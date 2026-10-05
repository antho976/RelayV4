//! Pointer and key translation for the mirror, kept free of widgets so it unit-tests.
//!
//! Input used to be fragile in two ways: every touch-move was its own request awaited in turn,
//! and a full queue ended the mirror ("input could not keep up"). Now moves that pile up behind
//! a slow request collapse into the newest one, downs and ups are never dropped, and nothing in
//! here can end a mirror.
use gtk4::gdk;
use serde_json::{json, Value};

/// Android motion actions, as `device.mirror.input` takes them.
pub const DOWN: u8 = 0;
pub const UP: u8 = 1;
pub const MOVE: u8 = 2;

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

pub fn touch(action: u8, (x, y): (i32, i32), (w, h): (i32, i32)) -> Value {
    let pressure = if action == UP { 0.0 } else { 1.0 };
    json!({"type":"touch","action":action,"x":x,"y":y,"w":w,"h":h,"pressure":pressure})
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

/// Keys with an Android meaning, sent as separate down and up events so holding one repeats
/// the way it does on a hardware keyboard. Printable text goes as text instead.
pub fn keycode(key: gdk::Key) -> Option<u32> {
    use gdk::Key;
    Some(match key {
        Key::Return | Key::KP_Enter | Key::ISO_Enter => 66,
        Key::BackSpace => 67,
        Key::Delete | Key::KP_Delete => 112,
        Key::Tab | Key::ISO_Left_Tab => 61,
        // Escape leaves the current screen, as it does on the desktop. It used to, too.
        Key::Escape => 4,
        Key::Left | Key::KP_Left => 21,
        Key::Right | Key::KP_Right => 22,
        Key::Up | Key::KP_Up => 19,
        Key::Down | Key::KP_Down => 20,
        Key::Home | Key::KP_Home => 122,
        Key::End | Key::KP_End => 123,
        Key::Page_Up | Key::KP_Page_Up => 92,
        Key::Page_Down | Key::KP_Page_Down => 93,
        Key::Insert => 124,
        Key::Menu => 82,
        Key::AudioRaiseVolume => 24,
        Key::AudioLowerVolume => 25,
        Key::AudioMute => 164,
        Key::AudioPlay | Key::AudioPause => 85,
        Key::AudioNext => 87,
        Key::AudioPrev => 88,
        _ => return None,
    })
}

/// Letters, digits and space as Android keys. Only used with Ctrl or Alt held — plain typing
/// goes as text, which respects the device's keyboard layout and IME — so shortcuts such as
/// Ctrl+A or Ctrl+Z reach the app as the key combination they are.
pub fn chord_keycode(key: gdk::Key) -> Option<u32> {
    let c = key.to_lower().to_unicode()?;
    match c {
        'a'..='z' => Some(29 + (c as u32 - 'a' as u32)),
        '0'..='9' => Some(7 + (c as u32 - '0' as u32)),
        ' ' => Some(62),
        _ => None,
    }
}

/// Android meta state for the held modifiers (AMETA_*).
pub fn meta(modifiers: gdk::ModifierType) -> u32 {
    let mut meta = 0;
    if modifiers.contains(gdk::ModifierType::SHIFT_MASK) {
        meta |= 0x1;
    }
    if modifiers.contains(gdk::ModifierType::ALT_MASK) {
        meta |= 0x2;
    }
    if modifiers.contains(gdk::ModifierType::CONTROL_MASK) {
        meta |= 0x1000;
    }
    meta
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
    }

    #[test]
    fn wheel_and_touchpad_scroll_normalize_to_notches() {
        assert_eq!(scroll_amount(0.0, 1.0, false), (0.0, -1.0));
        assert_eq!(scroll_amount(0.0, -3.0, false), (0.0, 1.0));
        assert_eq!(scroll_amount(12.0, 0.0, true), (0.5, -0.0));
    }

    #[test]
    fn editing_and_navigation_keys_have_android_codes() {
        assert_eq!(keycode(gdk::Key::Return), Some(66));
        assert_eq!(keycode(gdk::Key::BackSpace), Some(67));
        assert_eq!(keycode(gdk::Key::Delete), Some(112));
        assert_eq!(keycode(gdk::Key::Home), Some(122));
        assert_eq!(keycode(gdk::Key::End), Some(123));
        assert_eq!(keycode(gdk::Key::Page_Down), Some(93));
        assert_eq!(keycode(gdk::Key::a), None);
        assert_eq!(keycode(gdk::Key::Escape), Some(4));
        assert_eq!(chord_keycode(gdk::Key::a), Some(29));
        assert_eq!(chord_keycode(gdk::Key::Z), Some(54));
        assert_eq!(chord_keycode(gdk::Key::_0), Some(7));
        assert_eq!(chord_keycode(gdk::Key::F5), None);
        assert_eq!(meta(gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::CONTROL_MASK), 0x1001);
    }
}
