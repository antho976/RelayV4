//! The video half of the mirror: base64 packets in, RGB pictures out, through one FFmpeg child.
//!
//! The decoder's command line, the packet gate and the picture reader are GTK-free and live in
//! `relay_client::mirror::decode`, with the measurements behind the FFmpeg line.
use super::Control;
use crate::client::Notice;
use base64::Engine as _;
pub use relay_client::mirror::decode::Pixels;
use relay_client::mirror::decode::{decoder_bytes, ffmpeg_args, ppm, Admit, Gate};
use std::process::Stdio;
use tokio::io::{AsyncWriteExt, BufReader};

/// Runs on the tokio runtime for one mirror session. Video packets go to FFmpeg; pictures come
/// back on `frames` (newest two kept — a slow paint skips pictures, never queues them); status
/// objects, resync requests and the reason the stream ended go to `control`, which is never
/// lossy. Returns when the engine reports a terminal state or anything breaks.
pub async fn run(
    notices: async_channel::Receiver<Notice>,
    frames: async_channel::Sender<Pixels>,
    stale: async_channel::Receiver<Pixels>,
    control: async_channel::Sender<Control>,
) {
    let result = pump(notices, frames, stale, &control).await;
    if let Err(message) = result {
        let _ = control.send(Control::Broken(message)).await;
    }
}

async fn pump(
    notices: async_channel::Receiver<Notice>,
    frames: async_channel::Sender<Pixels>,
    stale: async_channel::Receiver<Pixels>,
    control: &async_channel::Sender<Control>,
) -> Result<(), String> {
    let mut decoder = tokio::process::Command::new("ffmpeg")
        .args(ffmpeg_args())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("{FFMPEG_MISSING} ({e})"))?;
    let mut input = decoder.stdin.take().ok_or("The video decoder has no input.")?;
    let mut output = BufReader::new(decoder.stdout.take().ok_or("The video decoder has no output.")?);
    let read = async {
        loop {
            let picture = ppm(&mut output).await?;
            if frames.is_full() {
                let _ = stale.try_recv();
            }
            if frames.try_send(picture).is_err() && frames.is_closed() {
                return Ok::<(), String>(());
            }
        }
    };
    let write = async {
        let mut gate = Gate::default();
        while let Ok(notice) = notices.recv().await {
            match notice {
                Notice::Frame(frame) if frame.stream == "mirror" => {
                    if let Some(encoded) = frame.data.as_str() {
                        let packet = base64::engine::general_purpose::STANDARD
                            .decode(encoded)
                            .map_err(|e| format!("The engine sent an unreadable video packet: {e}"))?;
                        let config = packet.first().is_some_and(|flags| flags & 1 != 0);
                        match gate.admit(frame.seq, config) {
                            Admit::Write => {
                                if let Some(bytes) = decoder_bytes(&packet) {
                                    input.write_all(&bytes).await.map_err(|_| "The video decoder stopped.".to_string())?;
                                }
                            }
                            Admit::Skip => {}
                            Admit::Reset => {
                                let _ = control.send(Control::Resync).await;
                            }
                        }
                    } else {
                        let terminal = matches!(frame.data["state"].as_str(), Some("stopped" | "failed" | "lost"));
                        let _ = control.send(Control::Status(frame.data)).await;
                        if terminal {
                            return Ok(());
                        }
                    }
                }
                Notice::Disconnected(error) => {
                    return Err(format!("Relay's engine connection closed ({error})."));
                }
                _ => {}
            }
        }
        Err("Relay's engine connection closed.".into())
    };
    let result = tokio::select! { result = read => result, result = write => result };
    let _ = decoder.kill().await;
    let _ = decoder.wait().await;
    result
}

pub const FFMPEG_MISSING: &str = "FFmpeg is needed to show the device screen. Install the ffmpeg package, then retry.";
