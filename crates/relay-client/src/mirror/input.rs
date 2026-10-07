//! Pointer translation and the `device.mirror.input` payloads, kept free of widgets so they
//! unit-test and so the engine's `tests/native_input.rs` sends exactly what the client sends.
//!
//! Input used to be fragile in two ways: every touch-move was its own request awaited in turn,
//! and a full queue ended the mirror ("input could not keep up"). Now moves that pile up behind
//! a slow request collapse into the newest one, downs and ups are never dropped, and nothing in
//! here can end a mirror.
use serde_json::{json, Value};

/// Android motion actions, as `device.mirror.input` takes them. Keys use DOWN and UP too.
pub const DOWN: u8 = 0;
pub const UP: u8 = 1;
pub const MOVE: u8 = 2;

/// The rail's keys in its three groups, top to bottom: `(glyph, caption, kind)`. A click sends
/// [`button`] with the kind, which is the engine's lowercased event type.
pub const RAIL: [[(&str, &str, &str); 3]; 3] = [
    [("power", "Power button", "power"), ("volume-up", "Volume up", "volumeup"), ("volume-down", "Volume down", "volumedown")],
    [("back", "Back (right-click, Esc)", "back"), ("home", "Home (middle-click)", "home"), ("recents", "Recent apps", "appswitch")],
    [("rotate", "Rotate the device", "rotate"), ("notifications", "Open notifications", "notifications"), ("quick-settings", "Open quick settings", "quicksettings")],
];

/// Map a point in the screen widget to device pixels, given the picture's size. The picture is
/// drawn letterboxed (`ContentFit::Contain`), so the same scale applies to both axes and the
/// bars are split evenly. `None` when the point is outside the picture itself — a press that
/// starts on a letterbox bar is not a touch.
pub fn map_point(area: (f64, f64), picture: (i32, i32), x: f64, y: f64) -> Option<(i32, i32)> {
    let (w, h) = picture;
    if w <= 0 || h <= 0 {
        return None;
    }
    let scale = (area.0 / w as f64).min(area.1 / h as f64);
    if scale <= 0.0 {
        return None;
    }
    let px = (x - (area.0 - w as f64 * scale) / 2.0) / scale;
    let py = (y - (area.1 - h as f64 * scale) / 2.0) / scale;
    if px < -0.5 || py < -0.5 || px > w as f64 - 0.5 || py > h as f64 - 0.5 {
        return None;
    }
    Some(clamp_point(picture, px, py))
}

/// Same mapping, but a point past the edge pins to it: a drag that leaves the picture keeps
/// tracking along its border and still releases where the finger "left" the screen.
pub fn map_point_clamped(area: (f64, f64), picture: (i32, i32), x: f64, y: f64) -> Option<(i32, i32)> {
    let (w, h) = picture;
    if w <= 0 || h <= 0 {
        return None;
    }
    let scale = (area.0 / w as f64).min(area.1 / h as f64);
    if scale <= 0.0 {
        return None;
    }
    let px = (x - (area.0 - w as f64 * scale) / 2.0) / scale;
    let py = (y - (area.1 - h as f64 * scale) / 2.0) / scale;
    Some(clamp_point(picture, px, py))
}

fn clamp_point((w, h): (i32, i32), x: f64, y: f64) -> (i32, i32) {
    (x.round().clamp(0.0, (w - 1) as f64) as i32, y.round().clamp(0.0, (h - 1) as f64) as i32)
}

/// One finger event at a device point; `w`/`h` are the stream size.
pub fn touch(action: u8, (x, y): (i32, i32), (w, h): (i32, i32)) -> Value {
    let pressure = if action == UP { 0.0 } else { 1.0 };
    json!({"type":"touch","action":action,"x":x,"y":y,"w":w,"h":h,"pressure":pressure})
}

/// A wheel or touchpad scroll at a device point, deltas from [`scroll_amount`].
pub fn scroll((x, y): (i32, i32), (w, h): (i32, i32), hscroll: f32, vscroll: f32) -> Value {
    json!({"type":"scroll","x":x,"y":y,"w":w,"h":h,"hscroll":hscroll,"vscroll":vscroll})
}

/// A key going down or up with the held modifiers' Android meta state.
pub fn key(action: u8, keycode: u32, meta: u32) -> Value {
    json!({"type":"key","action":action,"keycode":keycode,"meta":meta})
}

/// One typed character, sent as text so the device's layout and IME apply.
pub fn text(c: char) -> Value {
    json!({"type":"text","text":c.to_string()})
}

/// The desktop clipboard, set on the device and pasted into the focused field.
pub fn paste(text: &str) -> Value {
    json!({"type":"setclipboard","text":text,"paste":true})
}

/// An event that is only its type: a rail key's kind from [`RAIL`], or `back` / `home` for the
/// secondary and middle mouse buttons.
pub fn button(kind: &str) -> Value {
    json!({"type":kind})
}

/// The display key: the device panel off (`false`) or back on, mirroring either way.
pub fn display_power(on: bool) -> Value {
    json!({"type":"displaypower","on":on})
}

/// Ask the device for a fresh codec config and key frame after the decoder lost the stream.
pub fn reset_video() -> Value {
    json!({"type":"resetvideo"})
}

fn is_move(event: &Value) -> bool {
    event["type"] == "touch" && event["action"] == MOVE
}

/// Collapse a burst of queued events: a touch-move immediately followed by another touch-move
/// is superseded by it. Everything else — downs, ups, keys, scrolls — passes through in order,
/// so a gesture always lands with its true start and end.
pub fn coalesce(events: Vec<Value>) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::with_capacity(events.len());
    for event in events {
        if is_move(&event) && out.last().is_some_and(is_move) {
            *out.last_mut().unwrap() = event;
        } else {
            out.push(event);
        }
    }
    out
}

/// Scroll deltas as scrcpy wants them: one wheel notch is ±1, positive vertical scrolls the
/// content *up* (GTK's dy is positive toward the user, so it flips). Touchpads report surface
/// pixels; roughly 24 px make a notch.
pub fn scroll_amount(dx: f64, dy: f64, pixels: bool) -> (f32, f32) {
    let unit = if pixels { 24.0 } else { 1.0 };
    let h = (dx / unit).clamp(-1.0, 1.0) as f32;
    let v = (-dy / unit).clamp(-1.0, 1.0) as f32;
    (h, v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_map_through_the_letterbox_and_bars_are_not_touches() {
        // A 464×1024 picture in a 600×1024 area: 68 px bars left and right.
        assert_eq!(map_point((600.0, 1024.0), (464, 1024), 68.0, 0.0), Some((0, 0)));
        assert_eq!(map_point((600.0, 1024.0), (464, 1024), 300.0, 512.0), Some((232, 512)));
        assert_eq!(map_point((600.0, 1024.0), (464, 1024), 20.0, 512.0), None);
        // Half size: every widget pixel is two device pixels.
        assert_eq!(map_point((232.0, 512.0), (464, 1024), 116.0, 256.0), Some((232, 512)));
        // A drag past the edge pins to it.
        assert_eq!(map_point_clamped((600.0, 1024.0), (464, 1024), 590.0, 2000.0), Some((463, 1023)));
        assert_eq!(map_point((0.0, 0.0), (464, 1024), 1.0, 1.0), None);
        assert_eq!(map_point((600.0, 1024.0), (0, 0), 1.0, 1.0), None);
    }

    #[test]
    fn queued_moves_collapse_but_downs_and_ups_survive() {
        let size = (464, 1024);
        let events = vec![
            touch(DOWN, (1, 1), size),
            touch(MOVE, (2, 2), size),
            touch(MOVE, (3, 3), size),
            touch(MOVE, (4, 4), size),
            touch(UP, (4, 4), size),
            json!({"type":"keypress","keycode":3}),
            touch(DOWN, (9, 9), size),
            touch(MOVE, (10, 10), size),
        ];
        let out = coalesce(events);
        let actions: Vec<_> = out.iter().map(|e| (e["type"].as_str().unwrap().to_string(), e["action"].as_u64(), e["x"].as_i64())).collect();
        assert_eq!(
            actions,
            vec![
                ("touch".into(), Some(0), Some(1)),
                ("touch".into(), Some(2), Some(4)),
                ("touch".into(), Some(1), Some(4)),
                ("keypress".into(), None, None),
                ("touch".into(), Some(0), Some(9)),
                ("touch".into(), Some(2), Some(10)),
            ]
        );
        assert_eq!(touch(UP, (0, 0), size)["pressure"], 0.0);
        // Keys and scrolls between moves keep both moves.
        let out = coalesce(vec![touch(MOVE, (1, 1), size), key(DOWN, 66, 0), touch(MOVE, (2, 2), size), scroll((2, 2), size, 0.0, 1.0), touch(MOVE, (3, 3), size)]);
        assert_eq!(out.len(), 5);
    }

    #[test]
    fn wheel_and_touchpad_scroll_normalize_to_notches() {
        assert_eq!(scroll_amount(0.0, 1.0, false), (0.0, -1.0));
        assert_eq!(scroll_amount(0.0, -3.0, false), (0.0, 1.0));
        assert_eq!(scroll_amount(12.0, 0.0, true), (0.5, -0.0));
    }

    #[test]
    fn payloads_keep_the_field_names_and_types_the_engine_reads() {
        assert_eq!(scroll((5, 7), (464, 1024), 0.5, -1.0), json!({"type":"scroll","x":5,"y":7,"w":464,"h":1024,"hscroll":0.5,"vscroll":-1.0}));
        assert_eq!(key(UP, 66, 0x1000), json!({"type":"key","action":1,"keycode":66,"meta":0x1000}));
        assert_eq!(text('é'), json!({"type":"text","text":"é"}));
        assert_eq!(paste("x"), json!({"type":"setclipboard","text":"x","paste":true}));
        assert_eq!(display_power(false), json!({"type":"displaypower","on":false}));
        assert_eq!(reset_video(), json!({"type":"resetvideo"}));
        let kinds: Vec<_> = RAIL.iter().flatten().map(|(_, _, kind)| button(kind)["type"].clone()).collect();
        assert_eq!(kinds.len(), 9);
        assert!(kinds.contains(&json!("appswitch")) && kinds.contains(&json!("quicksettings")));
    }
}
