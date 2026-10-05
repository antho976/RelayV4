//! Live Phase 10 device streams. Runtime transports are bounded and exist only while a
//! mirror or run is active; durable run metadata lives in `device_runs`.

use relay_bus::types::Id;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::{broadcast, watch};

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
    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
    pub fn install(&self, mut child: Child) -> bool {
        let mut slot = self.child.lock().unwrap();
        if self.stopped() {
            let _ = child.kill();
            return false;
        }
        *slot = Some(child);
        true
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(child) = self.child.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
    }
    pub fn finish(&self) {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.wait();
        }
    }
}

/// Point the engine at a bundled scrcpy server. Native development also has a source-tree fallback.
pub fn configure_mirror_server(path: PathBuf) {
    if path.is_file() {
        let _ = MIRROR_SERVER.set(path);
    }
}

pub fn mirror_server_path() -> Option<PathBuf> {
    MIRROR_SERVER.get().cloned().or_else(|| {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/relay-native/resources/scrcpy-server-v4.1");
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

/// Where a mirror is in its life. `Stopped`, `Failed` and `Lost` are terminal: the
/// runtime never leaves them, and the socket door ends the window's stream on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MirrorState {
    #[default]
    Starting,
    Running,
    /// Someone asked for it to end (`device.mirror.stop`, or the window went away).
    Stopped,
    /// The transport or the server failed while the device was still there.
    Failed,
    /// The device itself went away — unplugged, rebooted, adb lost it.
    Lost,
}

impl MirrorState {
    pub fn is_terminal(self) -> bool {
        matches!(self, MirrorState::Stopped | MirrorState::Failed | MirrorState::Lost)
    }
}

/// What a window needs to draw the mirror's state: the live picture size, the device's
/// own name once the handshake gave it, and on a terminal state the typed code and the
/// message to show next to Retry. Forwarded verbatim as a `mirror` frame whose `data` is
/// this object (video frames carry a base64 string instead).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct MirrorStatus {
    pub state: MirrorState,
    pub width: u32,
    pub height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

fn pack_size(width: u32, height: u32) -> u64 {
    (u64::from(width) << 32) | u64::from(height)
}

#[derive(Debug)]
pub struct MirrorRuntime {
    pub id: Id,
    pub device: String,
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
    /// The stream's current picture size, `width << 32 | height`. Starts at the estimate
    /// from `wm size` and follows every session packet, so tap/swipe coordinates are
    /// checked and scaled against what the device is actually sending — after a rotation
    /// too, which is exactly when the estimate is wrong.
    size: AtomicU64,
    status: watch::Sender<MirrorStatus>,
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
        let (status, _) = watch::channel(MirrorStatus { width, height, ..MirrorStatus::default() });
        Arc::new(Self {
            id,
            device,
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
            size: AtomicU64::new(pack_size(width, height)),
            status,
        })
    }
    /// The picture size the device is sending right now.
    pub fn size(&self) -> (u32, u32) {
        let packed = self.size.load(Ordering::Relaxed);
        ((packed >> 32) as u32, packed as u32)
    }
    /// A session packet: the stream changed size (start, rotation, resize). Updates the
    /// size inputs are checked against and, while running, the status windows draw from.
    pub fn set_size(&self, width: u32, height: u32) {
        self.size.store(pack_size(width, height), Ordering::Relaxed);
        self.status.send_if_modified(|status| {
            if status.state.is_terminal() || (status.width, status.height) == (width, height) {
                return false;
            }
            status.width = width;
            status.height = height;
            true
        });
    }
    pub fn status(&self) -> MirrorStatus {
        self.status.borrow().clone()
    }
    /// A receiver that already holds the current status, so an attach that arrives after a
    /// fast failure still sees it rather than waiting for a change that already happened.
    pub fn watch_status(&self) -> watch::Receiver<MirrorStatus> {
        self.status.subscribe()
    }
    /// The handshake finished: the device's name is known and frames are about to flow.
    pub fn set_running(&self, name: String) {
        let (width, height) = self.size();
        self.status.send_if_modified(|status| {
            if status.state.is_terminal() {
                return false;
            }
            *status = MirrorStatus { state: MirrorState::Running, width, height, name: Some(name), code: None, message: None };
            true
        });
    }
    /// Enter a terminal state. The first one wins — a stop that races a failure must not
    /// relabel a device loss as a clean stop, or the reverse.
    pub fn finish(&self, state: MirrorState, code: Option<String>, message: Option<String>) -> bool {
        debug_assert!(state.is_terminal());
        self.status.send_if_modified(|status| {
            if status.state.is_terminal() {
                return false;
            }
            status.state = state;
            status.code = code;
            status.message = message;
            true
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
