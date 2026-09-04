//! Live Phase 10 device streams. Runtime transports are bounded and exist only while a
//! mirror or run is active; durable run metadata lives in `device_runs`.

use relay_bus::types::Id;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::broadcast;

static MIRROR_SERVER: OnceLock<PathBuf> = OnceLock::new();
const RUN_LOG_LINES: usize = 512;

#[derive(Debug, Default)]
pub struct DeviceWatchRuntime {
    stop: AtomicBool,
    child: Mutex<Option<Child>>,
}

#[derive(Debug, Default)]
pub struct DeviceWatchState {
    pub clients: usize,
    pub runtime: Option<Arc<DeviceWatchRuntime>>,
}

impl DeviceWatchRuntime {
    pub fn stopped(&self) -> bool { self.stop.load(Ordering::Relaxed) }
    pub fn install(&self, mut child: Child) -> bool {
        let mut slot = self.child.lock().unwrap();
        if self.stopped() { let _ = child.kill(); return false; }
        *slot = Some(child);
        true
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(child) = self.child.lock().unwrap().as_mut() { let _ = child.kill(); }
    }
    pub fn finish(&self) {
        if let Some(mut child) = self.child.lock().unwrap().take() { let _ = child.wait(); }
    }
}

/// Point the headless device runtime at the Tauri-bundled scrcpy server. The source-tree
/// fallback below keeps tests and `tauri dev` self-contained.
pub fn configure_mirror_server(path: PathBuf) {
    if path.is_file() {
        let _ = MIRROR_SERVER.set(path);
    }
}

pub fn mirror_server_path() -> Option<PathBuf> {
    MIRROR_SERVER.get().cloned().or_else(|| {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/relay-app/src-tauri/resources/scrcpy-server-v4.1");
        path.is_file().then_some(path)
    })
}

#[derive(Debug, Clone)]
pub struct MirrorChunk {
    pub seq: u64,
    pub data: Arc<Vec<u8>>,
}

#[derive(Debug)]
struct MirrorBuffer {
    seq: u64,
    bytes: usize,
    chunks: VecDeque<MirrorChunk>,
}

#[derive(Debug, Default)]
struct MirrorControl {
    socket: Option<TcpStream>,
    pending: VecDeque<Vec<u8>>,
}

#[derive(Debug)]
pub struct MirrorRuntime {
    pub id: Id,
    pub device: String,
    pub width: u32,
    pub height: u32,
    pub input_width: u32,
    pub input_height: u32,
    pub max_size: u32,
    pub bitrate: u32,
    pub scid: u32,
    pub adb: String,
    stop: AtomicBool,
    child: Mutex<Option<Child>>,
    video: Mutex<Option<TcpStream>>,
    control: Mutex<MirrorControl>,
    buffer: Mutex<MirrorBuffer>,
    tx: broadcast::Sender<MirrorChunk>,
}

pub struct MirrorRuntimeConfig {
    pub id: Id,
    pub device: String,
    pub width: u32,
    pub height: u32,
    pub input_width: u32,
    pub input_height: u32,
    pub max_size: u32,
    pub bitrate: u32,
    pub scid: u32,
    pub adb: String,
}

impl MirrorRuntime {
    pub fn new(config: MirrorRuntimeConfig) -> Arc<Self> {
        let MirrorRuntimeConfig {
            id,
            device,
            width,
            height,
            input_width,
            input_height,
            max_size,
            bitrate,
            scid,
            adb,
        } = config;
        let (tx, _) = broadcast::channel(256);
        Arc::new(Self {
            id,
            device,
            width,
            height,
            input_width,
            input_height,
            max_size,
            bitrate,
            scid,
            adb,
            stop: AtomicBool::new(false),
            child: Mutex::new(None),
            video: Mutex::new(None),
            control: Mutex::new(MirrorControl::default()),
            buffer: Mutex::new(MirrorBuffer {
                seq: 0,
                bytes: 0,
                chunks: VecDeque::new(),
            }),
            tx,
        })
    }
    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
    pub fn set_child(&self, child: Child) {
        *self.child.lock().unwrap() = Some(child);
    }
    pub fn take_child(&self) -> Option<Child> {
        self.child.lock().unwrap().take()
    }
    pub fn set_video(&self, video: TcpStream) {
        *self.video.lock().unwrap() = Some(video);
    }
    pub fn install_control(&self, mut socket: TcpStream) -> io::Result<()> {
        let mut control = self.control.lock().unwrap();
        while let Some(bytes) = control.pending.pop_front() {
            socket.write_all(&bytes)?;
        }
        control.socket = Some(socket);
        Ok(())
    }
    pub fn send_control(&self, bytes: Vec<u8>) -> io::Result<()> {
        let mut control = self.control.lock().unwrap();
        if let Some(socket) = control.socket.as_mut() {
            socket.write_all(&bytes)
        } else {
            if control.pending.len() == 128 {
                control.pending.pop_front();
            }
            control.pending.push_back(bytes);
            Ok(())
        }
    }
    pub fn push(&self, data: Vec<u8>) {
        if data.is_empty() || self.stopped() {
            return;
        }
        let chunk = {
            let mut state = self.buffer.lock().unwrap();
            state.seq += 1;
            let data = Arc::new(data);
            let chunk = MirrorChunk {
                seq: state.seq,
                data: data.clone(),
            };
            state.bytes += data.len();
            state.chunks.push_back(chunk.clone());
            while state.bytes > 2 * 1024 * 1024 {
                if let Some(old) = state.chunks.pop_front() {
                    state.bytes = state.bytes.saturating_sub(old.data.len());
                } else {
                    break;
                }
            }
            chunk
        };
        let _ = self.tx.send(chunk);
    }
    pub fn attach(&self) -> (u64, Vec<MirrorChunk>, broadcast::Receiver<MirrorChunk>) {
        let rx = self.tx.subscribe();
        let state = self.buffer.lock().unwrap();
        (state.seq, state.chunks.iter().cloned().collect(), rx)
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(video) = self.video.lock().unwrap().take() {
            let _ = video.shutdown(std::net::Shutdown::Both);
        }
        if let Some(control) = self.control.lock().unwrap().socket.take() {
            let _ = control.shutdown(std::net::Shutdown::Both);
        }
        if let Some(child) = self.child.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
    }
}

#[derive(Debug, Clone)]
pub struct LogLine {
    pub seq: u64,
    pub line: Arc<String>,
}

#[derive(Debug)]
struct LogBuffer {
    seq: u64,
    lines: VecDeque<LogLine>,
}

#[derive(Debug)]
pub struct RunRuntime {
    pub id: Id,
    stop: AtomicBool,
    child: Mutex<Option<Child>>,
    buffer: Mutex<LogBuffer>,
    tx: broadcast::Sender<LogLine>,
}

impl RunRuntime {
    pub fn new(id: Id) -> Arc<Self> {
        let (tx, _) = broadcast::channel(RUN_LOG_LINES);
        Arc::new(Self {
            id,
            stop: AtomicBool::new(false),
            child: Mutex::new(None),
            buffer: Mutex::new(LogBuffer {
                seq: 0,
                lines: VecDeque::new(),
            }),
            tx,
        })
    }
    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
    pub fn set_child(&self, child: Child) {
        *self.child.lock().unwrap() = Some(child);
    }
    pub fn take_child(&self) -> Option<Child> {
        self.child.lock().unwrap().take()
    }
    pub fn push(&self, line: impl Into<String>) {
        if self.stopped() {
            return;
        }
        let value = {
            let mut state = self.buffer.lock().unwrap();
            state.seq += 1;
            let value = LogLine {
                seq: state.seq,
                line: Arc::new(line.into()),
            };
            state.lines.push_back(value.clone());
            while state.lines.len() > RUN_LOG_LINES {
                state.lines.pop_front();
            }
            value
        };
        let _ = self.tx.send(value);
    }
    pub fn attach(&self) -> (u64, Vec<LogLine>, broadcast::Receiver<LogLine>) {
        let rx = self.tx.subscribe();
        let state = self.buffer.lock().unwrap();
        (state.seq, state.lines.iter().cloned().collect(), rx)
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(child) = self.child.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_history_keeps_only_the_recent_tail() {
        let runtime = RunRuntime::new(1);
        for line in 1..=600 {
            runtime.push(format!("line {line}"));
        }
        let (cursor, history, _) = runtime.attach();
        assert_eq!(cursor, 600);
        assert_eq!(history.len(), RUN_LOG_LINES);
        assert_eq!(history.first().unwrap().seq, 89);
        assert_eq!(history.last().unwrap().seq, 600);
    }
}
