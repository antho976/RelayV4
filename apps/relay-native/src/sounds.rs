use crate::app::Ui;
use serde_json::{json, Value};
use std::{process::Stdio, rc::Rc, time::Duration};
use tokio::io::AsyncWriteExt;

pub fn play(ui: &Rc<Ui>, kind: &str, volume: f64) {
    if kind == "off" || volume <= 0. || ui.sound_busy.replace(true) {
        return;
    }
    let bytes = samples(kind, volume);
    let worker = ui.rt.spawn(async move {
        play_command(notification_command(), &bytes, Duration::from_secs(3)).await
    });
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        if let Err(e) = worker
            .await
            .unwrap_or_else(|e| Err(std::io::Error::other(e.to_string())))
        {
            ui.show_error(&format!("Notification sound: {e}"));
        }
        ui.sound_busy.set(false);
    });
}

fn notification_command() -> tokio::process::Command {
    let mut command = tokio::process::Command::new("ffplay");
    command.args([
        "-nodisp",
        "-autoexit",
        "-loglevel",
        "error",
        "-f",
        "f32le",
        "-ar",
        "22050",
        "-ch_layout",
        "mono",
        "-i",
        "pipe:0",
    ]);
    command
}

/// Bound the entire write/play/wait cycle so a disconnected audio device cannot
/// leave notification sounds permanently busy. Keep the backend error for diagnosis.
async fn play_command(
    mut command: tokio::process::Command,
    bytes: &[u8],
    timeout: Duration,
) -> std::io::Result<()> {
    tokio::time::timeout(timeout, async {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let mut stdin = child.stdin.take().expect("piped audio input");
        // Drain stderr while writing, including when the backend rejects the input.
        let (write, output) = tokio::join!(
            async {
                stdin.write_all(bytes).await?;
                drop(stdin);
                Ok::<_, std::io::Error>(())
            },
            child.wait_with_output(),
        );
        let output = output?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr);
            return Err(std::io::Error::other(format!(
                "Audio playback failed ({}): {}",
                output.status,
                detail.trim().chars().take(400).collect::<String>()
            )));
        }
        write
    })
    .await
    .map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "Audio playback timed out after waiting for the sound device",
        )
    })?
}

pub fn notify(ui: &Rc<Ui>, event: &Value) {
    if std::env::var("RELAY_NATIVE_FIXTURE").as_deref() == Ok("1") {
        return;
    }
    let category = event["category"].as_str().unwrap_or("system").to_string();
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        if let Ok(settings) = ui.call("notify.settings.get", json!({})).await {
            if settings["categories"][&category] != false {
                play(
                    &ui,
                    settings["sound"].as_str().unwrap_or("chime"),
                    settings["volume"].as_f64().unwrap_or(0.7),
                );
            }
        }
    });
}

fn samples(kind: &str, volume: f64) -> Vec<u8> {
    let notes: &[(f64, f64, f64)] = match kind {
        "glass" => &[(1046., 0., 0.06), (1568., 0.055, 0.12)],
        "pulse" => &[(330., 0., 0.08), (440., 0.085, 0.08)],
        "signal" => &[(523., 0., 0.055), (659., 0.07, 0.055), (784., 0.14, 0.09)],
        _ => &[(660., 0., 0.09), (880., 0.1, 0.13)],
    };
    let gain = 0.34 * volume.clamp(0., 1.).powi(3);
    (0..5513)
        .flat_map(|n| {
            let time = n as f64 / 22050.;
            let value = notes
                .iter()
                .map(|&(freq, offset, duration)| {
                    let t = time - offset;
                    if t < 0. || t > duration {
                        return 0.;
                    }
                    let envelope = if t < 0.012 {
                        t / 0.012
                    } else {
                        ((-8. * (t - 0.012)) / (duration - 0.012)).exp()
                    };
                    let wave = (std::f64::consts::TAU * freq * t).sin();
                    (if kind == "pulse" {
                        wave.asin() * 2. / std::f64::consts::PI
                    } else {
                        wave
                    }) * envelope
                        * gain
                })
                .sum::<f64>() as f32;
            value.to_le_bytes()
        })
        .collect()
}

#[test]
fn sound_samples_are_short_finite_and_volume_scaled() {
    for kind in ["chime", "glass", "pulse", "signal"] {
        let bytes = samples(kind, 1.);
        assert_eq!(bytes.len(), 22052);
        assert!(bytes.as_chunks::<4>().0.iter().all(|b| {
            let v = f32::from_le_bytes(*b);
            v.is_finite() && v.abs() <= 0.68
        }));
        assert!(samples(kind, 0.).iter().all(|b| *b == 0 || *b == 128));
    }
}

#[tokio::test]
async fn audio_backend_errors_are_reported_and_hangs_are_bounded() {
    let mut failed = tokio::process::Command::new("sh");
    failed.args([
        "-c",
        "cat >/dev/null; echo 'device unavailable' >&2; exit 1",
    ]);
    let error = play_command(failed, &samples("chime", 0.), Duration::from_secs(2))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("device unavailable"));

    let mut hung = tokio::process::Command::new("sleep");
    hung.arg("30");
    let start = std::time::Instant::now();
    let error = play_command(hung, &[0; 100_000], Duration::from_millis(100))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
#[ignore = "requires the installed ffplay and a live audio server"]
async fn installed_notification_backend_accepts_silent_audio() {
    play_command(
        notification_command(),
        &samples("chime", 0.),
        Duration::from_secs(3),
    )
    .await
    .unwrap();
}
