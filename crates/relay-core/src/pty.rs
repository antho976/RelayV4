//! PTYs (SPEC §1): one per spawned session. A detached reader thread feeds a scrollback ring
//! and a broadcast of frames (BUS.md §7: `epoch` per spawn, `seq` per frame). Teardown is
//! kill child → drop master → the reader ends on EIO, in that order, always.

use anyhow::{anyhow, Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

/// Scrollback kept per PTY. Bytes, not lines: what the terminal would replay.
pub const SCROLLBACK_BYTES: usize = 8 * 1024 * 1024;
/// Maximum bytes sent when a subscriber catches up. The larger ring remains available for
/// explicit scrollback inspection and persistence, but one stale pane must not monopolize the UI.
pub const ATTACH_CATCHUP_BYTES: usize = 256 * 1024;
/// How many recent frame boundaries we remember for `attach {from_seq}` replay.
const FRAME_INDEX: usize = 8192;

#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub cmd: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: PathBuf,
    pub cols: u16,
    pub rows: u16,
    pub epoch: u64,
    /// Prior epochs collapsed into the new terminal's initial history on resume/wake.
    pub initial_scrollback: Vec<u8>,
}

/// One frame of output as it left the PTY.
#[derive(Debug, Clone)]
pub struct Frame {
    pub epoch: u64,
    pub seq: u64,
    pub data: Vec<u8>,
}

struct Ring {
    buf: VecDeque<u8>,
    /// Absolute byte offset of `buf[0]` since spawn.
    base: u64,
    /// (seq, absolute offset where that frame starts), oldest first.
    frames: VecDeque<(u64, u64)>,
}

impl Ring {
    /// Append one frame and trim both rings back to their caps. Trimming is a single bulk
    /// `drain`, never a per-element `pop_front` loop: a provider that writes 64 KiB at a time
    /// would otherwise pay 65 536 individual ring operations for every read once the byte ring
    /// is full, which is the whole cost of the terminal data plane at steady state.
    fn push(&mut self, seq: u64, data: &[u8]) {
        let start = self.base + self.buf.len() as u64;
        self.frames.push_back((seq, start));
        if self.frames.len() > FRAME_INDEX {
            let excess = self.frames.len() - FRAME_INDEX;
            self.frames.drain(..excess);
        }
        // `extend` from a slice copies in bulk; `data.iter().copied()` would not.
        self.buf.extend(data);
        if self.buf.len() > SCROLLBACK_BYTES {
            let excess = self.buf.len() - SCROLLBACK_BYTES;
            self.buf.drain(..excess);
            self.base += excess as u64;
        }
    }
    /// Bytes from absolute offset `from` (clamped to what we still have). Copied through the
    /// deque's two contiguous halves — an element-wise iterator over 8 MiB is the same answer
    /// an order of magnitude slower.
    fn bytes_from(&self, from: u64) -> Vec<u8> {
        let skip = from.saturating_sub(self.base).min(self.buf.len() as u64) as usize;
        let (head, tail) = self.buf.as_slices();
        let mut out = Vec::with_capacity(self.buf.len() - skip);
        if skip < head.len() {
            out.extend_from_slice(&head[skip..]);
            out.extend_from_slice(tail);
        } else {
            out.extend_from_slice(&tail[skip - head.len()..]);
        }
        out
    }
    /// The first frame boundary after `seq`, if we still know it. Both `seq` and the offsets
    /// increase with every push, so the index is sorted and a scan is never needed.
    fn offset_after(&self, seq: u64) -> Option<u64> {
        let at = self.frames.partition_point(|(s, _)| *s <= seq);
        self.frames.get(at).map(|(_, offset)| *offset)
    }
    fn end(&self) -> u64 {
        self.base + self.buf.len() as u64
    }
    fn bounded_bytes_from(&self, from: u64) -> Vec<u8> {
        let earliest = self.end().saturating_sub(ATTACH_CATCHUP_BYTES as u64);
        let requested = from.max(earliest);
        let at = self.frames.partition_point(|(_, offset)| *offset < requested);
        let aligned = self.frames.get(at).map(|(_, offset)| *offset).unwrap_or(requested);
        self.bytes_from(aligned)
    }
    /// The bytes covering the last `lines` newline-terminated lines, so reading a short tail
    /// out of an 8 MiB ring costs the tail rather than the ring.
    fn tail_lines(&self, lines: usize) -> Vec<u8> {
        let (head, tail) = self.buf.as_slices();
        let mut seen = 0usize;
        let mut start = None;
        // Walk backwards through the two halves, stopping at the newline that opens the
        // (lines)-th line from the end.
        'scan: for (part, base) in [(tail, head.len()), (head, 0)] {
            for (i, byte) in part.iter().enumerate().rev() {
                if *byte == b'\n' {
                    seen += 1;
                    if seen > lines {
                        start = Some(base + i + 1);
                        break 'scan;
                    }
                }
            }
        }
        let start = start.unwrap_or(0);
        self.bytes_from(self.base + start as u64)
    }
}

struct Shared {
    epoch: u64,
    seq: AtomicU64,
    /// Unix milliseconds of the last byte the child produced, 0 if it has produced none.
    /// Kept in memory and read on demand: this is how Relay sees a provider that runs no
    /// lifecycle hooks at all, without a timer anywhere (SPEC §15, D108).
    last_output_ms: AtomicU64,
    ring: Mutex<Ring>,
    tx: broadcast::Sender<Arc<Frame>>,
    exited: AtomicBool,
    exit_code: Mutex<Option<i32>>,
    /// Mirror of `sessions.state == 'idle'`, kept here so `session.input` can decide whether a
    /// keystroke needs a database write without reading the database (D148). Only the
    /// idle→running edge does; every other keystroke is pure memory.
    idle: AtomicBool,
}

/// A live PTY. Cheap to clone the handle (`Arc`).
pub struct Pty {
    pid: u32,
    shared: Arc<Shared>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    started: Instant,
}

/// What `attach` hands a subscriber: catch-up bytes, where they end, and the live feed.
pub struct Attached {
    pub epoch: u64,
    pub seq: u64,
    pub catch_up: Vec<u8>,
    pub rx: broadcast::Receiver<Arc<Frame>>,
}

impl Pty {
    /// Spawn `spec` on a fresh PTY. `on_exit` runs on a detached thread once the child is
    /// reaped (with its exit code, `None` if killed by signal).
    pub fn spawn(spec: SpawnSpec, on_exit: impl FnOnce(Option<i32>) + Send + 'static) -> Result<Arc<Pty>> {
        let sys = native_pty_system();
        let pair = sys
            .openpty(PtySize { rows: spec.rows.max(2), cols: spec.cols.max(2), pixel_width: 0, pixel_height: 0 })
            .map_err(|e| anyhow!("openpty: {e}"))?;
        let mut cmd = CommandBuilder::new(&spec.cmd);
        cmd.args(&spec.args);
        cmd.cwd(&spec.cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        // The daemon may have been started by a noninteractive, monochrome tool.
        // Its log preference must not disable colors in an interactive agent PTY.
        cmd.env_remove("NO_COLOR");
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        let mut child = pair.slave.spawn_command(cmd).with_context(|| format!("spawning {}", spec.cmd))?;
        drop(pair.slave);
        let pid = child.process_id().ok_or_else(|| anyhow!("spawned child has no pid"))?;
        let mut reader = pair.master.try_clone_reader().map_err(|e| anyhow!("clone reader: {e}"))?;
        let writer = pair.master.take_writer().map_err(|e| anyhow!("take writer: {e}"))?;

        let (tx, _) = broadcast::channel(1024);
        let mut ring = Ring { buf: VecDeque::new(), base: 0, frames: VecDeque::new() };
        if !spec.initial_scrollback.is_empty() {
            ring.push(0, &spec.initial_scrollback);
        }
        let shared = Arc::new(Shared {
            epoch: spec.epoch,
            seq: AtomicU64::new(0),
            last_output_ms: AtomicU64::new(0),
            ring: Mutex::new(ring),
            tx,
            idle: AtomicBool::new(false),
            exited: AtomicBool::new(false),
            exit_code: Mutex::new(None),
        });

        // reader: detached; ends when the master is dropped (EIO) or the child closes the slave
        let sh = shared.clone();
        std::thread::Builder::new().name(format!("pty-read-{pid}")).spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        sh.last_output_ms.store(unix_millis(), Ordering::Relaxed);
                        let data = buf[..n].to_vec();
                        // `seq` advances under the ring lock, in the same critical section as
                        // the push. `attach` reads both while holding that lock, so the seq it
                        // reports is always the last frame its catch-up bytes contain — bumping
                        // the counter first let it label a catch-up "through N" that ended at
                        // N-1, and the forwarder then dropped frame N as already delivered.
                        let seq = {
                            let mut ring = sh.ring.lock().unwrap();
                            let seq = sh.seq.fetch_add(1, Ordering::SeqCst) + 1;
                            ring.push(seq, &data);
                            seq
                        };
                        let _ = sh.tx.send(Arc::new(Frame { epoch: sh.epoch, seq, data }));
                    }
                }
            }
        })?;

        // waiter: reaps the child, records the exit code, runs the callback
        let sh = shared.clone();
        std::thread::Builder::new().name(format!("pty-wait-{pid}")).spawn(move || {
            let code = child.wait().ok().map(|s| s.exit_code() as i32);
            *sh.exit_code.lock().unwrap() = code;
            sh.exited.store(true, Ordering::SeqCst);
            on_exit(code);
        })?;

        Ok(Arc::new(Pty {
            pid,
            shared,
            master: Mutex::new(Some(pair.master)),
            writer: Mutex::new(Some(writer)),
            started: Instant::now(),
        }))
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }
    pub fn epoch(&self) -> u64 {
        self.shared.epoch
    }
    pub fn seq(&self) -> u64 {
        self.shared.seq.load(Ordering::SeqCst)
    }
    /// When the child last wrote anything, as an RFC 3339 timestamp. `None` before its first
    /// byte. True for every provider, hooked or not.
    pub fn last_output_at(&self) -> Option<String> {
        let millis = self.shared.last_output_ms.load(Ordering::Relaxed);
        (millis > 0).then(|| {
            jiff::Timestamp::from_millisecond(millis as i64)
                .unwrap_or(jiff::Timestamp::UNIX_EPOCH)
                .to_string()
        })
    }
    pub fn exited(&self) -> bool {
        self.shared.exited.load(Ordering::SeqCst)
    }
    /// Record what the session row now says, so the input fast path knows whether it owes the
    /// store a state change.
    pub fn set_idle(&self, idle: bool) {
        self.shared.idle.store(idle, Ordering::SeqCst);
    }
    /// Claim the idle→running edge: true for the first caller after [`Pty::set_idle(true)`],
    /// false for every keystroke after it. Exactly one deferred write per idle period.
    pub fn claim_idle_edge(&self) -> bool {
        self.shared.idle.swap(false, Ordering::SeqCst)
    }
    pub fn exit_code(&self) -> Option<i32> {
        *self.shared.exit_code.lock().unwrap()
    }
    pub fn uptime(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn write(&self, data: &[u8]) -> Result<()> {
        let mut w = self.writer.lock().unwrap();
        let w = w.as_mut().ok_or_else(|| anyhow!("pty is closed"))?;
        w.write_all(data)?;
        w.flush()?;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        let m = self.master.lock().unwrap();
        let m = m.as_ref().ok_or_else(|| anyhow!("pty is closed"))?;
        m.resize(PtySize { rows: rows.max(2), cols: cols.max(2), pixel_width: 0, pixel_height: 0 })
            .map_err(|e| anyhow!("resize: {e}"))
    }

    /// Subscribe with bounded catch-up since (`from_epoch`, `from_seq`) plus the live feed.
    pub fn attach(&self, from_epoch: Option<u64>, from_seq: Option<u64>) -> Attached {
        let rx = self.shared.tx.subscribe();
        let ring = self.shared.ring.lock().unwrap();
        // Read under the ring lock: the reader thread bumps the counter and pushes the frame
        // in one critical section, so this seq names exactly the last frame in `ring`.
        let seq = self.seq();
        let catch_up = match (from_epoch, from_seq) {
            (Some(e), Some(s)) if e == self.shared.epoch => {
                if s >= seq { Vec::new() } else { ring.bounded_bytes_from(ring.offset_after(s).unwrap_or(0)) }
            }
            _ => ring.bounded_bytes_from(0),
        };
        drop(ring);
        Attached { epoch: self.shared.epoch, seq, catch_up, rx }
    }

    /// The scrollback as text (last `lines` if given) and where it ends. A bounded request
    /// reads only the tail it asks for: the ring holds 8 MiB, and `session.scrollback {lines}`
    /// is a UI-facing read that must not copy all of it to keep twenty lines.
    pub fn scrollback(&self, lines: Option<usize>) -> (String, u64, u64) {
        let ring = self.shared.ring.lock().unwrap();
        let bytes = match lines {
            Some(n) => ring.tail_lines(n),
            None => ring.bytes_from(0),
        };
        drop(ring);
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let text = match lines {
            Some(n) => {
                let v: Vec<&str> = text.lines().collect();
                let start = v.len().saturating_sub(n);
                v[start..].join("\n")
            }
            None => text,
        };
        (text, self.shared.epoch, self.seq())
    }

    /// Kill the child (SIGTERM to its process group, then SIGKILL after `grace`), then drop
    /// the master so the reader ends. Idempotent.
    pub fn kill(&self, grace: Duration) {
        if !self.exited() {
            unsafe {
                // the child is a session leader on its own pty, so -pid reaches its children
                libc::kill(-(self.pid as i32), libc::SIGTERM);
                libc::kill(self.pid as i32, libc::SIGTERM);
            }
            let deadline = Instant::now() + grace;
            while !self.exited() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            if !self.exited() {
                unsafe {
                    libc::kill(-(self.pid as i32), libc::SIGKILL);
                    libc::kill(self.pid as i32, libc::SIGKILL);
                }
                let deadline = Instant::now() + Duration::from_secs(2);
                while !self.exited() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
        // drop writer then master: the reader thread sees EIO and ends
        self.writer.lock().unwrap().take();
        self.master.lock().unwrap().take();
    }
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or(0)
}



impl Drop for Pty {
    fn drop(&mut self) {
        // last handle gone: make sure nothing lingers
        if !self.exited() {
            unsafe {
                libc::kill(-(self.pid as i32), libc::SIGKILL);
                libc::kill(self.pid as i32, libc::SIGKILL);
            }
        }
    }
}

/// Is `pid` alive? (signal 0)
pub fn pid_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// Environment of a live process, from /proc.
pub fn proc_env(pid: u32) -> Option<Vec<(String, String)>> {
    let raw = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    Some(raw.split(|b| *b == 0).filter_map(|kv| {
        let s = String::from_utf8_lossy(kv);
        let (k, v) = s.split_once('=')?;
        Some((k.to_string(), v.to_string()))
    }).collect())
}

/// Is `pid` a Relay child of *this* engine's store, spawned for `session`? Read from its
/// environment, the same way [`relay_children`] finds orphans. A pid recorded in the sessions
/// table outlives the process it named: after a reboot, or once the kernel has cycled through
/// its pid space, that number belongs to whatever process happens to hold it now, and the
/// number alone must never be reason to kill it.
pub fn owned_by_session(pid: u32, instance: &str, store: &str, session: &str) -> bool {
    let Some(env) = proc_env(pid) else { return false };
    let var = |key: &str| env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
    var("RELAY_INSTANCE") == Some(instance)
        && var("RELAY_STORE") == Some(store)
        && var("RELAY_SESSION") == Some(session)
}

/// Every live pid whose environment carries `RELAY_SESSION` for `instance` **and** this
/// store (`RELAY_STORE`) — Relay's own children (and theirs), regardless of which engine
/// process spawned them. The store filter keeps a test engine from reaping a real one.
pub fn relay_children(instance: &str, store: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir("/proc") else { return out };
    let me = std::process::id();
    for e in rd.flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
        if pid == me { continue; }
        let Some(env) = proc_env(pid) else { continue };
        let inst = env.iter().find(|(k, _)| k == "RELAY_INSTANCE").map(|(_, v)| v.as_str());
        let st = env.iter().find(|(k, _)| k == "RELAY_STORE").map(|(_, v)| v.as_str());
        let sess = env.iter().find(|(k, _)| k == "RELAY_SESSION").map(|(_, v)| v.clone());
        if inst == Some(instance) && st == Some(store) {
            if let Some(s) = sess {
                out.push((pid, s));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catch_up_is_bounded_and_starts_on_a_frame_boundary() {
        let mut ring = Ring { buf: VecDeque::new(), base: 0, frames: VecDeque::new() };
        for seq in 1..=1100 { ring.push(seq, &vec![(seq % 251) as u8; 1024]); }
        let catch_up = ring.bounded_bytes_from(0);
        assert_eq!(catch_up.len(), ATTACH_CATCHUP_BYTES);
        assert_eq!(catch_up.len() % 1024, 0);
    }

    fn ring() -> Ring {
        Ring { buf: VecDeque::new(), base: 0, frames: VecDeque::new() }
    }

    #[test]
    fn bulk_trimming_keeps_the_same_window_and_offsets_as_a_byte_at_a_time_ring() {
        let mut r = ring();
        let chunk = 64 * 1024;
        let pushes = (SCROLLBACK_BYTES / chunk) + 3;
        for seq in 1..=pushes as u64 {
            r.push(seq, &vec![(seq % 251) as u8; chunk]);
        }
        assert_eq!(r.buf.len(), SCROLLBACK_BYTES);
        assert_eq!(r.base, (pushes * chunk - SCROLLBACK_BYTES) as u64);
        assert_eq!(r.end(), (pushes * chunk) as u64);
        // Every byte still in the window is the one its absolute offset names.
        let all = r.bytes_from(0);
        assert_eq!(all.len(), SCROLLBACK_BYTES);
        let first_seq = (r.base as usize / chunk) as u64 + 1;
        assert_eq!(all[0], (first_seq % 251) as u8);
        assert_eq!(*all.last().unwrap(), (pushes as u64 % 251) as u8);
        // A partial read lands on the same byte the offset names.
        let tail = r.bytes_from(r.end() - 10);
        assert_eq!(tail, vec![(pushes as u64 % 251) as u8; 10]);
    }

    #[test]
    fn a_single_push_larger_than_the_ring_keeps_its_tail() {
        let mut r = ring();
        let huge = SCROLLBACK_BYTES + 4096;
        r.push(0, &vec![7u8; huge]);
        assert_eq!(r.buf.len(), SCROLLBACK_BYTES);
        assert_eq!(r.base, 4096);
        assert_eq!(r.end(), huge as u64);
    }

    #[test]
    fn frame_lookup_matches_a_linear_scan() {
        let mut r = ring();
        for seq in 1..=500u64 {
            r.push(seq, &[0u8; 16]);
        }
        for seq in 0..=501u64 {
            let scanned = r.frames.iter().find(|(s, _)| *s > seq).map(|(_, o)| *o);
            assert_eq!(r.offset_after(seq), scanned, "seq {seq}");
        }
    }

    #[test]
    fn a_bounded_tail_read_returns_the_same_lines_as_the_whole_ring() {
        let mut r = ring();
        for seq in 1..=400u64 {
            r.push(seq, format!("line {seq}\n").as_bytes());
        }
        let whole = String::from_utf8(r.bytes_from(0)).unwrap();
        for lines in [1usize, 5, 399, 400, 4000] {
            let tail = String::from_utf8(r.tail_lines(lines)).unwrap();
            let expect: Vec<&str> = whole.lines().collect();
            let expect = expect[expect.len().saturating_sub(lines)..].join("\n");
            let got: Vec<&str> = tail.lines().collect();
            let got = got[got.len().saturating_sub(lines)..].join("\n");
            assert_eq!(got, expect, "{lines} lines");
        }
    }

    #[test]
    fn a_tail_read_survives_a_wrapped_ring_without_splitting_a_line() {
        let mut r = ring();
        let filler = vec![b'z'; 1024];
        let mut seq = 0u64;
        while r.end() < SCROLLBACK_BYTES as u64 + 2048 {
            seq += 1;
            let mut line = filler.clone();
            line.push(b'\n');
            r.push(seq, &line);
        }
        let tail = String::from_utf8(r.tail_lines(3)).unwrap();
        assert_eq!(tail.lines().count(), 3);
        assert!(tail.lines().all(|l| l.len() == 1024));
    }
}
