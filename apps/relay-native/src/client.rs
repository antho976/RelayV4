//! Bounded, concurrent socket transport. Each visible terminal gets its own connection,
//! so terminal backpressure cannot starve the application's control plane.
use relay_bus::envelope::{Event, Frame, Request, Response, ENVELOPE_V};
use relay_bus::{Actor, BusError};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};
use tokio::task::AbortHandle;
use uuid::Uuid;

type Pending = Arc<Mutex<HashMap<Uuid, oneshot::Sender<Result<Value, Error>>>>>;
const QUEUE: usize = 64;
const MAX_LINE: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Bus(Box<BusError>),
    #[error("Connection closed. Reconnect to the engine.")]
    Disconnected,
    #[error("Request timed out. Its outcome is unknown; refresh before retrying.")]
    Timeout,
    #[error("Invalid engine response: {0}")]
    Protocol(String),
    #[error("{0}")]
    Io(String),
}

impl From<BusError> for Error {
    fn from(error: BusError) -> Self {
        Self::Bus(Box::new(error))
    }
}

#[derive(Debug)]
pub enum Notice {
    Event(Event),
    Frame(Frame),
    Disconnected(Error),
}

struct Connection {
    tx: mpsc::Sender<Request>,
    pending: Pending,
    tasks: Vec<AbortHandle>,
    notices: async_channel::Sender<Notice>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        self.pending.lock().unwrap().clear();
        self.notices.close();
    }
}

#[derive(Clone)]
pub struct Client(Arc<Connection>);

impl Client {
    pub async fn connect(
        rt: &Handle,
        path: PathBuf,
    ) -> Result<(Self, async_channel::Receiver<Notice>), Error> {
        rt.spawn(async move { Self::open(path).await })
            .await
            .map_err(|e| Error::Io(e.to_string()))?
    }

    async fn open(path: PathBuf) -> Result<(Self, async_channel::Receiver<Notice>), Error> {
        let socket = UnixStream::connect(&path)
            .await
            .map_err(|e| Error::Io(format!("Cannot connect to {}: {e}", path.display())))?;
        let (read, mut write) = socket.into_split();
        let pending: Pending = Arc::default();
        let (tx, mut rx) = mpsc::channel::<Request>(QUEUE);
        let (notices, receiver) = async_channel::bounded(QUEUE);
        let waiting = pending.clone();
        let reader_notices = notices.clone();
        let reader = tokio::spawn(async move {
            let mut reader = BufReader::new(read);
            let outcome: Result<(), Error> = async {
                loop {
                    // read_until alone has no size limit. Bound the frame before allocation.
                    let mut line = Vec::new();
                    loop {
                        let available = reader
                            .fill_buf()
                            .await
                            .map_err(|e| Error::Io(e.to_string()))?;
                        if available.is_empty() {
                            return Err(Error::Disconnected);
                        }
                        let n = available
                            .iter()
                            .position(|b| *b == b'\n')
                            .map_or(available.len(), |n| n + 1);
                        if line.len() + n > MAX_LINE {
                            return Err(Error::Protocol("frame exceeds 2 MiB".into()));
                        }
                        let complete = available[n - 1] == b'\n';
                        line.extend_from_slice(&available[..n]);
                        reader.consume(n);
                        if complete {
                            break;
                        }
                    }
                    let value: Value = serde_json::from_slice(&line)
                        .map_err(|e| Error::Protocol(e.to_string()))?;
                    if value["v"].as_u64() != Some(ENVELOPE_V.into()) {
                        return Err(Error::Protocol("unsupported envelope version".into()));
                    }
                    if value.get("ev").is_some() {
                        let event = serde_json::from_value(value)
                            .map_err(|e| Error::Protocol(e.to_string()))?;
                        reader_notices
                            .send(Notice::Event(event))
                            .await
                            .map_err(|_| Error::Disconnected)?;
                    } else if value.get("stream").is_some() {
                        let frame = serde_json::from_value(value)
                            .map_err(|e| Error::Protocol(e.to_string()))?;
                        reader_notices
                            .send(Notice::Frame(frame))
                            .await
                            .map_err(|_| Error::Disconnected)?;
                    } else {
                        let response: Response = serde_json::from_value(value)
                            .map_err(|e| Error::Protocol(e.to_string()))?;
                        let id = response
                            .id
                            .ok_or_else(|| Error::Protocol("response has no request id".into()))?;
                        if let Some(reply) = waiting.lock().unwrap().remove(&id) {
                            let _ = reply.send(response.into_result().map_err(Error::from));
                        }
                    }
                }
            }
            .await;
            let error = outcome.unwrap_err();
            for (_, reply) in waiting.lock().unwrap().drain() {
                let _ = reply.send(Err(error.clone()));
            }
            // Close before notifying so request callers and drains cannot wait forever.
            let _ = reader_notices.send(Notice::Disconnected(error)).await;
            reader_notices.close();
        });
        let waiting = pending.clone();
        let writer_notices = notices.clone();
        let reader_abort = reader.abort_handle();
        let writer = tokio::spawn(async move {
            while let Some(request) = rx.recv().await {
                let mut bytes = serde_json::to_vec(&request).expect("request serialization");
                bytes.push(b'\n');
                if let Err(e) = write.write_all(&bytes).await {
                    reader_abort.abort();
                    for (_, reply) in waiting.lock().unwrap().drain() {
                        let _ = reply.send(Err(Error::Disconnected));
                    }
                    let _ = writer_notices
                        .send(Notice::Disconnected(Error::Io(e.to_string())))
                        .await;
                    writer_notices.close();
                    break;
                }
            }
        });
        Ok((
            Self(Arc::new(Connection {
                tx,
                pending,
                tasks: vec![reader.abort_handle(), writer.abort_handle()],
                notices,
            })),
            receiver,
        ))
    }

    pub async fn request(&self, rt: &Handle, op: &str, payload: Value) -> Result<Value, Error> {
        if self.0.notices.is_closed() {
            return Err(Error::Disconnected);
        }
        let request = Request::new(Actor::User, op, payload);
        let id = request.id;
        let (send, reply) = oneshot::channel();
        self.0.pending.lock().unwrap().insert(id, send);
        // Clean up even if the GTK future is cancelled while awaiting an answer.
        struct Remove(Pending, Uuid);
        impl Drop for Remove {
            fn drop(&mut self) {
                self.0.lock().unwrap().remove(&self.1);
            }
        }
        let _remove = Remove(self.0.pending.clone(), id);
        let tx = self.0.tx.clone();
        let timeout = if op == "project.clone" {
            Duration::from_secs(1800)
        } else {
            Duration::from_secs(30)
        };
        rt.spawn(async move {
            tokio::time::timeout(timeout, async {
                tx.send(request).await.map_err(|_| Error::Disconnected)?;
                reply.await.map_err(|_| Error::Disconnected)?
            })
            .await
            .map_err(|_| Error::Timeout)?
        })
        .await
        .map_err(|_| Error::Disconnected)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::UnixListener;

    #[tokio::test]
    async fn correlates_out_of_order_replies_events_and_disconnect() {
        let path = std::env::temp_dir().join(format!("relay-native-{}.sock", Uuid::new_v4()));
        let server = UnixListener::bind(&path).unwrap();
        let rt = Handle::current();
        let (client, notices) = Client::connect(&rt, path.clone()).await.unwrap();
        let fixture = tokio::spawn(async move {
            let (socket, _) = server.accept().await.unwrap();
            let (read, mut write) = socket.into_split();
            let mut lines = BufReader::new(read).lines();
            let a: Request =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            let b: Request =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(a.actor, Actor::User);
            assert!(a.token.is_none());
            let event = serde_json::json!({"v":1,"ev":"task.changed","ts":"2026-09-04T00:00:00Z","actor":"user","payload":{}});
            for line in [
                event,
                serde_json::to_value(Response::ok(b.id, serde_json::json!(b.op))).unwrap(),
                serde_json::to_value(Response::ok(a.id, serde_json::json!(a.op))).unwrap(),
            ] {
                write
                    .write_all(format!("{line}\n").as_bytes())
                    .await
                    .unwrap();
            }
            let _: Request =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        });
        let (a, b) = tokio::join!(
            client.request(&rt, "a", Value::Null),
            client.request(&rt, "b", Value::Null)
        );
        assert_eq!(a.unwrap(), "a");
        assert_eq!(b.unwrap(), "b");
        assert!(matches!(notices.recv().await.unwrap(), Notice::Event(_)));
        assert!(client.request(&rt, "c", Value::Null).await.is_err());
        fixture.await.unwrap();
        assert!(client.0.pending.lock().unwrap().is_empty());
        std::fs::remove_file(path).unwrap();
    }
}
