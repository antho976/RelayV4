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

/// Which of the three line shapes a socket line carries.
enum Shape {
    Event,
    Frame,
    Response,
}

/// The shape of `line`, read from its leading bytes.
///
/// Every PTY frame the engine sends lands here, and a frame's payload is the bulk of the line,
/// so the cost that matters is how many times the line is scanned. Each envelope serializes its
/// fields in declaration order, which puts a distinguishing key first and lets the common case
/// be exactly one typed parse. `None` means "not recognizable from the prefix", and the caller
/// falls back to a `serde_json::Value` to decide — so a reordered field costs a little speed
/// and never correctness. A matched prefix has also proven the envelope version.
fn shape(line: &[u8]) -> Option<Shape> {
    let rest = line.strip_prefix(br#"{"v":1,""#)?;
    if rest.starts_with(br#"ev""#) {
        Some(Shape::Event)
    } else if rest.starts_with(br#"stream""#) {
        Some(Shape::Frame)
    } else if rest.starts_with(br#"id""#) {
        Some(Shape::Response)
    } else {
        None
    }
}

const QUEUE: usize = 64;
const MAX_LINE: usize = 2 * 1024 * 1024;

pub fn is_lifecycle_request(op: &str) -> bool {
    matches!(
        op,
        "session.create"
            | "session.close"
            | "session.park"
            | "session.wake"
            | "session.resume"
            | "session.spawn"
            | "session.clear_restorable"
            | "session.discard_restorable"
            | "task.dispatch"
    )
}

fn request_timeout(op: &str) -> Duration {
    match op {
        "project.clone" => Duration::from_secs(1800),
        // A new checkout may hydrate large LFS assets after the bounded fetch.
        "session.create" | "git.worktree.create" | "task.dispatch" => Duration::from_secs(180),
        _ => Duration::from_secs(30),
    }
}

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
    /// Binary file responses use a separate bounded connection. Keep the normal
    /// control/event channel's 2 MiB limit and responsiveness unchanged.
    pub async fn image_read(rt: &Handle, path: PathBuf, payload: Value) -> Result<Value, Error> {
        let (client, _notices) = rt
            .spawn(async move { Self::open_with_limit(path, 24 * 1024 * 1024).await })
            .await
            .map_err(|e| Error::Io(e.to_string()))??;
        client.request(rt, "file.read", payload).await
    }
    /// Session lifecycle actions must not sit behind slow reads on the UI socket.
    /// This connection has no subscriptions and exists only for this one action.
    pub async fn lifecycle_request(
        rt: &Handle,
        path: PathBuf,
        op: &str,
        payload: Value,
    ) -> Result<Value, Error> {
        let (client, _notices) = Self::connect(rt, path).await?;
        client.request(rt, op, payload).await
    }

    pub async fn connect(
        rt: &Handle,
        path: PathBuf,
    ) -> Result<(Self, async_channel::Receiver<Notice>), Error> {
        rt.spawn(async move { Self::open(path).await })
            .await
            .map_err(|e| Error::Io(e.to_string()))?
    }

    async fn open(path: PathBuf) -> Result<(Self, async_channel::Receiver<Notice>), Error> {
        Self::open_with_limit(path, MAX_LINE).await
    }

    async fn open_with_limit(
        path: PathBuf,
        max_line: usize,
    ) -> Result<(Self, async_channel::Receiver<Notice>), Error> {
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
                        if line.len() + n > max_line {
                            return Err(Error::Protocol(format!(
                                "frame exceeds {} MiB",
                                max_line / 1024 / 1024
                            )));
                        }
                        let complete = available[n - 1] == b'\n';
                        line.extend_from_slice(&available[..n]);
                        reader.consume(n);
                        if complete {
                            break;
                        }
                    }
                    let shape = match shape(&line) {
                        Some(shape) => shape,
                        None => {
                            let value: Value = serde_json::from_slice(&line)
                                .map_err(|e| Error::Protocol(e.to_string()))?;
                            if value["v"].as_u64() != Some(ENVELOPE_V.into()) {
                                return Err(Error::Protocol(
                                    "unsupported envelope version".into(),
                                ));
                            }
                            if value.get("ev").is_some() {
                                Shape::Event
                            } else if value.get("stream").is_some() {
                                Shape::Frame
                            } else {
                                Shape::Response
                            }
                        }
                    };
                    if matches!(shape, Shape::Event) {
                        let event = serde_json::from_slice(&line)
                            .map_err(|e| Error::Protocol(e.to_string()))?;
                        reader_notices
                            .send(Notice::Event(event))
                            .await
                            .map_err(|_| Error::Disconnected)?;
                    } else if matches!(shape, Shape::Frame) {
                        let frame = serde_json::from_slice(&line)
                            .map_err(|e| Error::Protocol(e.to_string()))?;
                        reader_notices
                            .send(Notice::Frame(frame))
                            .await
                            .map_err(|_| Error::Disconnected)?;
                    } else {
                        let response: Response = serde_json::from_slice(&line)
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
        self.request_with_id(rt, op, payload, Uuid::new_v4()).await
    }

    pub async fn request_with_id(
        &self,
        rt: &Handle,
        op: &str,
        payload: Value,
        id: Uuid,
    ) -> Result<Value, Error> {
        if self.0.notices.is_closed() {
            return Err(Error::Disconnected);
        }
        let mut request = Request::new(Actor::User, op, payload);
        request.id = id;
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
        let timeout = request_timeout(op);
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
    async fn lifecycle_actions_bypass_a_stalled_status_request() {
        for op in ["session.close", "session.create", "task.dispatch"] {
            lifecycle_bypasses_a_stalled_status_request(op).await;
        }
    }

    async fn lifecycle_bypasses_a_stalled_status_request(op: &'static str) {
        let path = std::env::temp_dir().join(format!("relay-lifecycle-{}.sock", Uuid::new_v4()));
        let server = UnixListener::bind(&path).unwrap();
        let rt = Handle::current();
        let (client, _notices) = Client::connect(&rt, path.clone()).await.unwrap();
        let (started, blocked) = oneshot::channel();
        let (release, released) = oneshot::channel();
        let fixture = tokio::spawn(async move {
            let (socket, _) = server.accept().await.unwrap();
            let (read, mut write) = socket.into_split();
            let mut lines = BufReader::new(read).lines();
            let slow: Request =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(slow.op, "git.status");
            started.send(()).unwrap();
            let (action, _) = server.accept().await.unwrap();
            let (read, mut action_write) = action.into_split();
            let mut lines = BufReader::new(read).lines();
            let close: Request =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(close.op, op);
            action_write
                .write_all(
                    format!(
                        "{}\n",
                        serde_json::to_string(&Response::ok(close.id, serde_json::json!({})))
                            .unwrap()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            released.await.unwrap();
            write
                .write_all(
                    format!(
                        "{}\n",
                        serde_json::to_string(&Response::ok(slow.id, serde_json::json!({})))
                            .unwrap()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let handle = rt.clone();
        let action_client = client.clone();
        let slow = tokio::spawn(async move {
            client
                .request(&handle, "git.status", serde_json::json!({}))
                .await
        });
        blocked.await.unwrap();
        tokio::time::timeout(Duration::from_millis(500), async {
            if is_lifecycle_request(op) {
                Client::lifecycle_request(
                    &rt,
                    path.clone(),
                    op,
                    serde_json::json!({"session":"fixture"}),
                )
                .await
            } else {
                action_client
                    .request(&rt, op, serde_json::json!({"session":"fixture"}))
                    .await
            }
        })
        .await
        .expect("Lifecycle action queued behind slow Git read")
        .unwrap();
        assert!(
            !slow.is_finished(),
            "The status request must still be blocked"
        );
        release.send(()).unwrap();
        slow.await.unwrap().unwrap();
        fixture.await.unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn correlates_out_of_order_replies_events_and_disconnect() {
        let path = std::env::temp_dir().join(format!("relay-native-{}.sock", Uuid::new_v4()));
        let server = UnixListener::bind(&path).unwrap();
        let rt = Handle::current();
        let (client, notices) = Client::connect(&rt, path.clone()).await.unwrap();
        let request_id = Uuid::new_v4();
        let fixture = tokio::spawn(async move {
            let (socket, _) = server.accept().await.unwrap();
            let (read, mut write) = socket.into_split();
            let mut lines = BufReader::new(read).lines();
            let a: Request =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            let b: Request =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            let tagged = if a.op == "a" { &a } else { &b };
            assert_eq!(tagged.id, request_id);
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
            client.request_with_id(&rt, "a", Value::Null, request_id),
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
