//! The video half of the mirror: base64 packets in, RGB pictures out, through one FFmpeg child.
//!
//! The FFmpeg line is measured, not guessed (scratchpad harness: a real 464×1024 H.264 stream
//! at 30 fps through a disposable engine, then a static screen):
//!
//! | input options                                   | AUD | median latency | frames never shown |
//! |-------------------------------------------------|-----|----------------|--------------------|
//! | `-probesize 32 -analyzeduration 0` (the old line) | no  | 535 ms         | 16 of 180          |
//! | `+ -fflags nobuffer -flags low_delay -threads 1`  | no  | 2039 ms        | 61                 |
//! | `+ -threads 1`                                    | no  | 34 ms          | 1                  |
//! | `+ -threads 1`, `-fps_mode passthrough`           | yes | 1 ms           | 0                  |
//!
//! Two separate holds were stacking. Frame threading keeps one picture per decoder thread in
//! flight (16 on a 16-core machine: 535 ms at 30 fps), and the raw H.264 parser cannot finish a
//! frame until the *next* one starts — so the last picture before the screen goes still never
//! appears at all. `-threads 1` removes the first; an access-unit delimiter written after each
//! packet ends the frame at once and removes the second. `-fflags nobuffer` is a trap here: it
//! makes this FFmpeg (n9) hold frames far longer, so it is deliberately absent.
use super::Control;
use crate::client::Notice;
use base64::Engine as _;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

/// H.264 access-unit delimiter (NAL type 9, primary_pic_type 7 = any slice type).
pub const AUD: [u8; 6] = [0, 0, 0, 1, 0x09, 0xf0];

pub fn ffmpeg_args() -> Vec<&'static str> {
    vec![
        "-hide_banner",
        "-loglevel",
        "error",
        "-probesize",
        "32",
        "-analyzeduration",
        "0",
        "-threads",
        "1",
        "-f",
        "h264",
        "-i",
        "pipe:0",
        "-an",
        "-fps_mode",
        "passthrough",
        "-f",
        "image2pipe",
        "-vcodec",
        "ppm",
        "-pix_fmt",
        "rgb24",
        "pipe:1",
    ]
}

/// What the decoder's stdin receives for one engine packet (`[flags][annex-b bytes]`, flag bit 0
/// = codec config). Config packets carry SPS/PPS and belong to the frame that follows them, so
/// only a picture gets the delimiter that closes it.
pub fn decoder_bytes(packet: &[u8]) -> Option<Vec<u8>> {
    let (flags, payload) = packet.split_first()?;
    if payload.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(payload.len() + AUD.len());
    out.extend_from_slice(payload);
    if flags & 1 == 0 {
        out.extend_from_slice(&AUD);
    }
    Some(out)
}

/// Decides, packet by packet, whether the decoder can take what arrives. A decoder cannot join
/// an H.264 stream mid-GOP, so after a sequence gap (the engine dropped packets for a slow
/// window) or a late join, it waits for the next codec config — and asks for one once.
#[derive(Debug)]
pub struct Gate {
    last: Option<u64>,
    waiting: bool,
    asked: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Admit {
    Write,
    Skip,
    /// Skip this packet and ask the device for a fresh config + key frame.
    Reset,
}

impl Default for Gate {
    fn default() -> Self {
        Self { last: None, waiting: true, asked: false }
    }
}

impl Gate {
    pub fn admit(&mut self, seq: u64, config: bool) -> Admit {
        if self.last.is_some_and(|last| seq != last + 1) {
            self.waiting = true;
            self.asked = false;
        }
        self.last = Some(seq);
        if config {
            self.waiting = false;
            self.asked = false;
            return Admit::Write;
        }
        if !self.waiting {
            return Admit::Write;
        }
        if self.asked {
            Admit::Skip
        } else {
            self.asked = true;
            Admit::Reset
        }
    }
}

pub struct Pixels {
    pub width: i32,
    pub height: i32,
    pub rgb: Vec<u8>,
}

pub async fn ppm<R: tokio::io::AsyncRead + Unpin>(read: &mut BufReader<R>) -> Result<Pixels, String> {
    async fn line<R: tokio::io::AsyncRead + Unpin>(read: &mut BufReader<R>) -> Result<String, String> {
        let mut value = String::new();
        let count = read.read_line(&mut value).await.map_err(|e| e.to_string())?;
        if count == 0 || value.len() > 64 {
            return Err("The video decoder stopped.".into());
        }
        Ok(value)
    }
    if line(read).await?.trim() != "P6" {
        return Err("The video decoder sent something other than an RGB picture.".into());
    }
    let size = line(read).await?;
    let mut sizes = size.split_whitespace();
    let mut dimension = || {
        sizes
            .next()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|n| *n > 0 && *n <= 4096)
            .ok_or("The video decoder reported an impossible picture size.")
    };
    let width = dimension()?;
    let height = dimension()?;
    if line(read).await?.trim() != "255" {
        return Err("The video decoder used an unsupported pixel depth.".into());
    }
    let mut rgb = vec![0; width * height * 3];
    read.read_exact(&mut rgb).await.map_err(|e| e.to_string())?;
    Ok(Pixels { width: width as i32, height: height as i32, rgb })
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn decoder_frames_preserve_rotation_and_reject_bad_dimensions() {
        let data = b"P6\n2 1\n255\nabcdefP6\n1 2\n255\nghijkl";
        let mut reader = BufReader::new(&data[..]);
        let a = ppm(&mut reader).await.unwrap();
        let b = ppm(&mut reader).await.unwrap();
        assert_eq!((a.width, a.height, a.rgb), (2, 1, b"abcdef".to_vec()));
        assert_eq!((b.width, b.height, b.rgb), (1, 2, b"ghijkl".to_vec()));
        let mut reader = BufReader::new(&b"P6\n9000 9000\n255\n"[..]);
        assert!(ppm(&mut reader).await.is_err());
    }

    #[test]
    fn the_decoder_line_is_the_measured_one() {
        let args = ffmpeg_args();
        let has = |pair: [&str; 2]| args.windows(2).any(|w| w == pair);
        assert!(has(["-threads", "1"]), "frame threading holds one picture per thread");
        assert!(has(["-fps_mode", "passthrough"]));
        // Input options must come before `-i` to apply to the decoder.
        let input = args.iter().position(|a| *a == "-i").unwrap();
        assert!(args.iter().position(|a| *a == "-threads").unwrap() < input);
        // Measured 4x worse than doing nothing; never add it back.
        assert!(!args.contains(&"nobuffer"));
    }

    #[test]
    fn pictures_get_a_delimiter_and_config_does_not() {
        let picture = [2u8, 0, 0, 0, 1, 0x65, 0xaa];
        let bytes = decoder_bytes(&picture).unwrap();
        assert_eq!(&bytes[..6], &picture[1..]);
        assert_eq!(&bytes[6..], &AUD);
        let config = [1u8, 0, 0, 0, 1, 0x67];
        assert_eq!(decoder_bytes(&config).unwrap(), config[1..].to_vec());
        assert!(decoder_bytes(&[0]).is_none());
        assert!(decoder_bytes(&[]).is_none());
    }

    #[test]
    fn a_gap_waits_for_the_next_config_and_asks_once() {
        let mut gate = Gate::default();
        // Normal start: config first, then pictures.
        assert_eq!(gate.admit(1, true), Admit::Write);
        assert_eq!(gate.admit(2, false), Admit::Write);
        // Packets 3..=9 were dropped for a slow window.
        assert_eq!(gate.admit(10, false), Admit::Reset);
        assert_eq!(gate.admit(11, false), Admit::Skip);
        assert_eq!(gate.admit(12, false), Admit::Skip);
        // The device answers the reset with config + key frame.
        assert_eq!(gate.admit(13, true), Admit::Write);
        assert_eq!(gate.admit(14, false), Admit::Write);
    }

    #[test]
    fn a_late_join_asks_for_config_instead_of_feeding_a_broken_stream() {
        let mut gate = Gate::default();
        assert_eq!(gate.admit(40, false), Admit::Reset);
        assert_eq!(gate.admit(41, false), Admit::Skip);
        assert_eq!(gate.admit(42, true), Admit::Write);
        // A gap that lands exactly on a config packet needs nothing.
        assert_eq!(gate.admit(50, true), Admit::Write);
        assert_eq!(gate.admit(51, false), Admit::Write);
    }
}
