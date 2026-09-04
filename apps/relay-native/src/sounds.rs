use crate::app::Ui;
use serde_json::{json, Value};
use std::{process::Stdio, rc::Rc};
use tokio::io::AsyncWriteExt;

pub fn play(ui: &Rc<Ui>, kind: &str, volume: f64) {
    if kind == "off" || volume <= 0. || ui.sound_busy.replace(true) {
        return;
    }
    let bytes = samples(kind, volume);
    let worker = ui.rt.spawn(async move {
        let mut child = tokio::process::Command::new("ffplay")
            .args([
                "-nodisp",
                "-autoexit",
                "-loglevel",
                "quiet",
                "-f",
                "f32le",
                "-ar",
                "22050",
                "-ac",
                "1",
                "-i",
                "pipe:0",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(&bytes).await?;
        }
        let status = child.wait().await?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other("Audio playback failed"))
        }
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
        assert!(bytes.chunks_exact(4).all(|b| {
            let v = f32::from_le_bytes(b.try_into().unwrap());
            v.is_finite() && v.abs() <= 0.68
        }));
        assert!(samples(kind, 0.).iter().all(|b| *b == 0 || *b == 128));
    }
}
