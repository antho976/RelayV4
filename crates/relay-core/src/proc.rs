//! Bounded subprocess execution.
//!
//! Handlers run with the store mutex held (BUS.md §5.1), so a child process that never returns
//! freezes the whole bus — including PTY keystrokes. Every subprocess Relay forks from inside a
//! handler goes through [`output_with_timeout`], which kills the child at the deadline and reports
//! a timeout the caller can turn into a typed refusal (D144).
//!
//! Long-running work belongs on a worker thread after commit, not here. This is for the short
//! probes — `adb devices`, `<provider> --version` — that are logically synchronous.

use std::io;
use std::process::{Command, ExitStatus, Output, Stdio};
use std::time::{Duration, Instant};

/// The longest the wait loop sleeps between checks on a child that has not exited yet.
const POLL_MAX: Duration = Duration::from_millis(5);
/// The first sleep. Most of what goes through here — `git status`, `adb devices`,
/// `<provider> --version` — finishes in single-digit milliseconds, and a flat 5 ms poll made
/// every one of them cost a 5 ms sleep it had already outlived. The interval doubles from here,
/// so a long-running child still settles onto the cheap `POLL_MAX` cadence.
const POLL_MIN: Duration = Duration::from_micros(150);

/// Run `cmd` to completion, or kill it once `timeout` elapses.
///
/// Returns `Ok(None)` when the child outlived the deadline. `stdin` is closed and both output
/// pipes are drained on their own threads, so a child that writes more than a pipe buffer can
/// never deadlock the wait.
pub fn output_with_timeout(cmd: &mut Command, timeout: Duration) -> io::Result<Option<Output>> {
    run(cmd, None, timeout)
}

/// [`output_with_timeout`] for a tool that reads an answer from stdin: `input` is written on
/// its own thread and stdin is then closed, so a child that never reads cannot block the wait.
pub fn output_with_input(cmd: &mut Command, input: &[u8], timeout: Duration) -> io::Result<Option<Output>> {
    run(cmd, Some(input.to_vec()), timeout)
}

fn run(cmd: &mut Command, input: Option<Vec<u8>>, timeout: Duration) -> io::Result<Option<Output>> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        std::thread::spawn(move || {
            let _ = io::Write::write_all(&mut stdin, &input);
        });
    }

    let pid = child.id();
    let stdout = child.stdout.take().map(drain);
    let stderr = child.stderr.take().map(drain);

    let deadline = Instant::now() + timeout;
    let mut interval = POLL_MIN;
    let status: Option<ExitStatus> = loop {
        match child.try_wait()? {
            Some(status) => break Some(status),
            None if Instant::now() < deadline => {
                std::thread::sleep(interval);
                interval = (interval * 2).min(POLL_MAX);
            }
            None => {
                #[cfg(unix)]
                unsafe { libc::kill(-(pid as i32), libc::SIGKILL); }
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };

    // A short-lived probe must not leave descendants holding its pipes open.
    #[cfg(unix)]
    unsafe { libc::kill(-(pid as i32), libc::SIGKILL); }
    let collect = |handle: Option<std::thread::JoinHandle<Vec<u8>>>| {
        handle.and_then(|h| h.join().ok()).unwrap_or_default()
    };
    let stdout = collect(stdout);
    let stderr = collect(stderr);
    Ok(status.map(|status| Output {
        status,
        stdout,
        stderr,
    }))
}

/// Make a network `git` command fail instead of waiting for a person: no terminal prompt, no
/// GUI askpass (an empty `GIT_ASKPASS` skips `core.askPass` and `SSH_ASKPASS` too), and a
/// transfer that stalls below 1 KB/s for a minute is abandoned. Credential helpers still run.
/// Call before adding the subcommand: the `-c` options belong to `git` itself.
pub fn quiet_network_git(cmd: &mut Command) {
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("GCM_INTERACTIVE", "never")
        .args(["-c", "http.lowSpeedLimit=1000", "-c", "http.lowSpeedTime=60"]);
}

fn drain<R: io::Read + Send + 'static>(mut reader: R) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = io::Read::read_to_end(&mut reader, &mut buf);
        buf
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_child_that_finishes_returns_its_output() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "printf hello"]);
        let out = output_with_timeout(&mut cmd, Duration::from_secs(5))
            .unwrap()
            .expect("the child exited well inside the deadline");
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout), "hello");
    }

    #[test]
    fn a_child_that_hangs_is_killed_at_the_deadline() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 30"]);
        let started = Instant::now();
        let out = output_with_timeout(&mut cmd, Duration::from_millis(150)).unwrap();
        assert!(out.is_none(), "a hung child must report a timeout");
        assert!(started.elapsed() < Duration::from_secs(5), "the wait must not outlive the deadline");
    }

    #[test]
    fn input_reaches_the_child_and_stdin_closes_after_it() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "read answer; echo got:$answer; cat"]);
        let out = output_with_input(&mut cmd, b"no\n", Duration::from_secs(5))
            .unwrap()
            .expect("stdin closes after the input, so `cat` sees EOF");
        assert_eq!(String::from_utf8_lossy(&out.stdout), "got:no\n");
    }

    #[test]
    fn output_larger_than_a_pipe_buffer_does_not_deadlock() {
        // 1 MiB is well past the 64 KiB pipe buffer: without the draining threads this hangs.
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "yes 0123456789 | head -c 1048576"]);
        let out = output_with_timeout(&mut cmd, Duration::from_secs(10))
            .unwrap()
            .expect("draining the pipes keeps the child from blocking on write");
        assert_eq!(out.stdout.len(), 1_048_576);
    }

    #[test]
    fn descendants_cannot_keep_probe_pipes_open_after_parent_exit() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 30 & echo done"]);
        let started = Instant::now();
        let out = output_with_timeout(&mut cmd, Duration::from_millis(150)).unwrap().unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(String::from_utf8_lossy(&out.stdout).contains("done"));
    }
}
