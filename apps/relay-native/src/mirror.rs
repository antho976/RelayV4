//! Native device mirror: engine-owned scrcpy transport, FFmpeg decode, GTK textures.
//! Only the newest two decoded images are retained; no idle refresh or unbounded video queue.
use crate::app::{button, label, Ui};
use crate::client::{Client, Notice};
use base64::Engine;
use gtk::prelude::*;
use gtk4 as gtk;
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::process::Stdio;
use std::rc::Rc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

// Overflow ends the mirror instead of losing a key or touch release.
#[derive(Clone)]
struct Input {
    queue: async_channel::Sender<serde_json::Value>,
    reset: async_channel::Sender<()>,
}
impl Input {
    fn try_send(
        &self,
        event: serde_json::Value,
    ) -> Result<(), async_channel::TrySendError<serde_json::Value>> {
        let result = self.queue.try_send(event);
        if result.is_err() {
            let _ = self.reset.try_send(());
        }
        result
    }
}

struct Pixels {
    width: i32,
    height: i32,
    rgb: Vec<u8>,
}
async fn ppm<R: tokio::io::AsyncRead + Unpin>(read: &mut BufReader<R>) -> Result<Pixels, String> {
    async fn line<R: tokio::io::AsyncRead + Unpin>(
        read: &mut BufReader<R>,
    ) -> Result<String, String> {
        let mut value = String::new();
        let count = read
            .read_line(&mut value)
            .await
            .map_err(|e| e.to_string())?;
        if count == 0 || value.len() > 64 {
            return Err("Video decoder closed or sent an invalid image header".into());
        }
        Ok(value)
    }
    if line(read).await?.trim() != "P6" {
        return Err("Expected an RGB image from decoder".into());
    }
    let size = line(read).await?;
    let mut sizes = size.split_whitespace();
    let width = sizes
        .next()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n > 0 && *n <= 4096)
        .ok_or("Invalid video width")?;
    let height = sizes
        .next()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n > 0 && *n <= 4096)
        .ok_or("Invalid video height")?;
    if line(read).await?.trim() != "255" {
        return Err("Unsupported video pixel depth".into());
    }
    let mut rgb = vec![0; width * height * 3];
    read.read_exact(&mut rgb).await.map_err(|e| e.to_string())?;
    Ok(Pixels {
        width: width as i32,
        height: height as i32,
        rgb,
    })
}

pub fn open(ui: &Rc<Ui>, device: String) {
    let window = gtk::Window::builder()
        .application(&ui.window.application().unwrap())
        .transient_for(&ui.window)
        .title(format!("Device · {device}"))
        .default_width(480)
        .default_height(850)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    window.set_child(Some(&root));
    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    toolbar.add_css_class("toolbar");
    root.append(&toolbar);
    let picture = gtk::Picture::new();
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk::ContentFit::Contain);
    root.append(&picture);
    let status = label("Starting mirror…", "dim");
    status.set_wrap(true);
    root.append(&status);
    let id = Rc::new(Cell::new(0i64));
    let dimensions = Rc::new(Cell::new((0i32, 0i32)));
    let (queue, event_rx) = async_channel::bounded::<serde_json::Value>(64);
    let (reset, reset_rx) = async_channel::bounded(1);
    let events = Input { queue, reset };
    for (caption, kind) in [
        ("Back", "back"),
        ("Home", "home"),
        ("Apps", "appswitch"),
        ("Rotate", "rotate"),
        ("Power", "power"),
        ("−", "volumedown"),
        ("+", "volumeup"),
    ] {
        let b = button(caption, "quiet");
        toolbar.append(&b);
        let events = events.clone();
        b.connect_clicked(move |_| {
            let _ = events.try_send(json!({"type":kind}));
        });
    }
    let texture = Rc::new(RefCell::new(None::<gtk::gdk::MemoryTexture>));
    let save = button("Capture", "quiet");
    toolbar.append(&save);
    let parent = window.clone();
    let tex = texture.clone();
    let message = status.clone();
    save.connect_clicked(move |_| {
        let Some(texture) = tex.borrow().clone() else {
            return;
        };
        let chooser = gtk::FileDialog::builder()
            .title("Save device screenshot")
            .initial_name("device.png")
            .build();
        let parent = parent.clone();
        let message = message.clone();
        glib::spawn_future_local(async move {
            if let Ok(file) = chooser.save_future(Some(&parent)).await {
                if let Some(path) = file.path() {
                    if let Err(e) = texture.save_to_png(path) {
                        message.set_text(&e.to_string());
                    }
                }
            }
        });
    });
    let gesture = gtk::GestureDrag::new();
    let p = picture.clone();
    let dims = dimensions.clone();
    let ev = events.clone();
    let origin = Rc::new(Cell::new(None));
    let start = origin.clone();
    gesture.connect_drag_begin(move |_, x, y| {
        if let Some((x, y, w, h)) = point(&p, dims.get(), x, y) {
            start.set(Some((x, y, w, h)));
            let _ = ev.try_send(
                json!({"type":"touch","action":0,"x":x,"y":y,"w":w,"h":h,"pressure":1.0}),
            );
        }
    });
    let p = picture.clone();
    let dims = dimensions.clone();
    let ev = events.clone();
    gesture.connect_drag_update(move |g, x, y| {
        if let Some((sx, sy)) = g.start_point() {
            if let Some((x, y, w, h)) = point(&p, dims.get(), sx + x, sy + y) {
                let _ = ev.try_send(
                    json!({"type":"touch","action":2,"x":x,"y":y,"w":w,"h":h,"pressure":1.0}),
                );
            }
        }
    });
    let p = picture.clone();
    let dims = dimensions.clone();
    let ev = events.clone();
    gesture.connect_drag_end(move |g, x, y| {
        if let Some((sx, sy)) = g.start_point() {
            if let Some((x, y, w, h)) = point(&p, dims.get(), sx + x, sy + y).or(origin.take()) {
                let _ = ev.try_send(
                    json!({"type":"touch","action":1,"x":x,"y":y,"w":w,"h":h,"pressure":0.0}),
                );
            }
        }
    });
    picture.add_controller(gesture);
    let keys = gtk::EventControllerKey::new();
    let ev = events.clone();
    let p = picture.clone();
    keys.connect_key_pressed(move |_, key, _, mods| {
        if mods.contains(gtk::gdk::ModifierType::CONTROL_MASK)
            && (key == gtk::gdk::Key::v || key == gtk::gdk::Key::V)
        {
            let clipboard = p.clipboard();
            let ev = ev.clone();
            glib::spawn_future_local(async move {
                if let Ok(Some(text)) = clipboard.read_text_future().await {
                    let _ = ev
                        .try_send(json!({"type":"setclipboard","text":text.as_str(),"paste":true}));
                }
            });
            return glib::Propagation::Stop;
        }
        let code = match key {
            gtk::gdk::Key::Return => Some(66),
            gtk::gdk::Key::BackSpace => Some(67),
            gtk::gdk::Key::Escape => Some(4),
            gtk::gdk::Key::Tab => Some(61),
            gtk::gdk::Key::Left => Some(21),
            gtk::gdk::Key::Right => Some(22),
            gtk::gdk::Key::Up => Some(19),
            gtk::gdk::Key::Down => Some(20),
            _ => None,
        };
        if let Some(keycode) = code {
            let _ = ev.try_send(json!({"type":"keypress","keycode":keycode}));
            return glib::Propagation::Stop;
        }
        if !mods.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
            if let Some(c) = key.to_unicode().filter(|c| !c.is_control()) {
                let _ = ev.try_send(json!({"type":"text","text":c.to_string()}));
                return glib::Propagation::Stop;
            }
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    let (rt, path) = (ui.rt.clone(), ui.path.clone());
    let worker_rt = rt.clone();
    let (frames, images) = async_channel::bounded::<Result<Pixels, String>>(2);
    let stale = images.clone();
    let mirror_id = id.clone();
    let task = glib::spawn_future_local(async move {
        let (client, notices) = match Client::connect(&rt, path).await {
            Ok(v) => v,
            Err(e) => {
                let _ = frames.send(Err(e.to_string())).await;
                return;
            }
        };
        match client
            .request(
                &rt,
                "device.mirror.start",
                json!({"device":device,"max_size":1024}),
            )
            .await
        {
            Ok(v) => mirror_id.set(v["mirror_id"].as_i64().unwrap_or(0)),
            Err(e) => {
                let _ = frames.send(Err(e.to_string())).await;
                return;
            }
        }
        let mid = mirror_id.get();
        let sender = frames.clone();
        let c = client.clone();
        let worker = worker_rt.spawn(async move {
            let mut decoder = tokio::process::Command::new("ffmpeg")
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-probesize",
                    "32",
                    "-analyzeduration",
                    "0",
                    "-f",
                    "h264",
                    "-i",
                    "pipe:0",
                    "-an",
                    "-f",
                    "image2pipe",
                    "-vcodec",
                    "ppm",
                    "-pix_fmt",
                    "rgb24",
                    "pipe:1",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| format!("FFmpeg is required for native mirroring: {e}"))?;
            let mut input = decoder.stdin.take().unwrap();
            let mut output = BufReader::new(decoder.stdout.take().unwrap());
            let read = async {
                loop {
                    let frame = ppm(&mut output).await?;
                    if sender.is_full() {
                        let _ = stale.try_recv();
                    }
                    if sender.try_send(Ok(frame)).is_err() && sender.is_closed() {
                        return Ok::<(), String>(());
                    }
                }
            };
            let write = async {
                let mut last = None;
                while let Ok(notice) = notices.recv().await {
                    match notice {
                        Notice::Frame(frame) if frame.stream == "mirror" => {
                            let encoded = frame.data.as_str().ok_or("Mirror stream interrupted")?;
                            if last.is_some_and(|seq| frame.seq != seq + 1) {
                                return Err(
                                    "Mirror lost video packets. Close and reopen it.".into()
                                );
                            }
                            last = Some(frame.seq);
                            let bytes = base64::engine::general_purpose::STANDARD
                                .decode(encoded)
                                .map_err(|e| e.to_string())?;
                            if bytes.len() > 1 {
                                input
                                    .write_all(&bytes[1..])
                                    .await
                                    .map_err(|e| e.to_string())?;
                            }
                        }
                        Notice::Disconnected(e) => return Err(e.to_string()),
                        _ => {}
                    }
                }
                Ok::<(), String>(())
            };
            let result = tokio::select! {result=read=>result,result=write=>result};
            let _ = decoder.kill().await;
            let _ = decoder.wait().await;
            result
        });
        let abort = worker.abort_handle();
        struct Stop(tokio::task::AbortHandle);
        impl Drop for Stop {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let _stop = Stop(abort);
        let input = async {
            while let Ok(event) = event_rx.recv().await {
                c.request(
                    &rt,
                    "device.mirror.input",
                    json!({"mirror_id":mid,"event":event}),
                )
                .await
                .map_err(|e| e.to_string())?;
            }
            Ok::<(), String>(())
        };
        let result = tokio::select! {value=worker=>value.map_err(|e|e.to_string()).and_then(|v|v),value=input=>value, _=reset_rx.recv()=>Err("Mirror stopped because input could not keep up. Reopen it to reconnect.".into())};
        if let Err(e) = result {
            let _ = frames.send(Err(e)).await;
        }
        let _ = client
            .request(&rt, "device.mirror.stop", json!({"mirror_id":mid}))
            .await;
    });
    let paint = glib::spawn_future_local(async move {
        while let Ok(frame) = images.recv().await {
            match frame {
                Ok(frame) => {
                    dimensions.set((frame.width, frame.height));
                    let bytes = glib::Bytes::from_owned(frame.rgb);
                    let t = gtk::gdk::MemoryTexture::new(
                        frame.width,
                        frame.height,
                        gtk::gdk::MemoryFormat::R8g8b8,
                        &bytes,
                        frame.width as usize * 3,
                    );
                    picture.set_paintable(Some(&t));
                    *texture.borrow_mut() = Some(t);
                    status.set_text("Drag to touch · Ctrl V to paste");
                }
                Err(e) => status.set_text(&e),
            }
        }
    });
    window.connect_close_request(move |_| {
        task.abort();
        paint.abort();
        glib::Propagation::Proceed
    });
    window.present();
}
fn point(p: &gtk::Picture, (w, h): (i32, i32), x: f64, y: f64) -> Option<(i32, i32, u16, u16)> {
    if w <= 0 || h <= 0 {
        return None;
    }
    let scale = (p.width() as f64 / w as f64).min(p.height() as f64 / h as f64);
    if scale <= 0.0 {
        return None;
    }
    let x = (x - (p.width() as f64 - w as f64 * scale) / 2.0) / scale;
    let y = (y - (p.height() as f64 - h as f64 * scale) / 2.0) / scale;
    Some((
        x.round().clamp(0.0, (w - 1) as f64) as i32,
        y.round().clamp(0.0, (h - 1) as f64) as i32,
        w as u16,
        h as u16,
    ))
}
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
}

#[test]
fn input_overflow_requests_device_reset() {
    let (queue, _events) = async_channel::bounded(1);
    let (reset, signal) = async_channel::bounded(1);
    let input = Input { queue, reset };
    input.try_send(json!({"type":"touch","action":0})).unwrap();
    assert!(input.try_send(json!({"type":"touch","action":1})).is_err());
    assert!(signal.try_recv().is_ok());
}
