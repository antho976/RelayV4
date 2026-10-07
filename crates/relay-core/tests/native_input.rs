//! The native client's `device.mirror.input` wire format, pinned on the engine side (RA-692).
//!
//! `apps/relay-native` cannot be built in CI (GTK 4.22), so the JSON its mirror sends would
//! otherwise never meet the engine in a test. Every payload below is built by the client's own
//! builders in `relay_client::mirror::input` (the headless half of `relay-native`'s mirror),
//! with the call site that sends it named beside it, so a payload change cannot drift past this
//! test. Each must be accepted and must reach the device's control socket as exactly the bytes
//! the matching `relay_core::mirror` encoder produces; if the engine stops accepting one, the
//! mirror broke.

mod common;

use common::ok;
use relay_core::engine::Engine;
use relay_client::mirror::input;
use relay_core::mirror::{self, ACTION_DOWN, ACTION_MOVE, ACTION_UP, META_NONE};
use serde_json::{json, Value};
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::time::Duration;

/// Just enough adb for `device.mirror.start` on the test instance, which starts no worker:
/// the device is listed and reports its panel size. (A subset of `bus.rs`'s fake.)
fn fake_adb(dir: &std::path::Path) -> String {
    let path = dir.join("adb");
    std::fs::write(&path, r#"#!/bin/sh
if [ "$1" = "devices" ]; then
  printf 'List of devices attached\nrelay-phone device product:relay model:Pixel_9_Pro device:relay transport_id:1\n'
  exit 0
fi
if [ "$3" = "shell" ] && [ "$4" = "wm" ] && [ "$5" = "size" ]; then
  printf 'Physical size: 1080x2400\n'
  exit 0
fi
exit 1
"#).unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).unwrap();
    path.display().to_string()
}

struct Mirror {
    engine: Arc<Engine>,
    _fixture: tempfile::TempDir,
    id: i64,
    /// The device's end of the control socket: what the engine writes, the server would read.
    control: TcpStream,
    size: (i64, i64),
}

/// A started mirror with a control socket installed, so each input is written as it is sent.
fn mirror() -> Mirror {
    let engine = common::engine();
    let fixture = tempfile::tempdir().unwrap();
    let adb = fake_adb(fixture.path());
    ok(&engine, "settings.set", json!({"path":"device.adb_path","value":adb}));
    let started = ok(&engine, "device.mirror.start", json!({"device":"relay-phone","max_size":1080,"bitrate":4_000_000}));
    let id = started["mirror_id"].as_i64().unwrap();
    let size = (started["width"].as_i64().unwrap(), started["height"].as_i64().unwrap());
    let runtime = relay_core::handlers::device::mirror_by_id(&engine, id).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    runtime.install_control(TcpStream::connect(listener.local_addr().unwrap()).unwrap()).unwrap();
    let (control, _) = listener.accept().unwrap();
    // A deadline, not a wait: a message that never comes fails the read instead of hanging it.
    control.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    Mirror { engine, _fixture: fixture, id, control, size }
}

impl Mirror {
    /// Send `event` as the client does and check the control socket receives exactly `expected`.
    /// A trailing `collapse` (which the client never sends) fences the message: a byte too many
    /// or too few shifts it, so the comparison fails rather than passing on a prefix.
    fn sends(&mut self, event: Value, expected: &[u8]) {
        ok(&self.engine, "device.mirror.input", json!({"mirror_id":self.id,"event":event.clone()}));
        ok(&self.engine, "device.mirror.input", json!({"mirror_id":self.id,"event":{"type":"collapse"}}));
        let mut want = expected.to_vec();
        want.extend_from_slice(&mirror::collapse_panels());
        let mut got = vec![0; want.len()];
        self.control.read_exact(&mut got).unwrap_or_else(|error| panic!("{event}: {error}"));
        assert_eq!(got, want, "{event}");
    }
}

#[test]
fn a_drag_lands_as_down_move_up_touches() {
    // `input::touch`, the client's only touch builder: `MirrorView::wire_pointer` sends it for
    // drag begin (DOWN), update (MOVE), end and cancel (UP). `w`/`h` are the stream size.
    let mut m = mirror();
    let (w, h) = (m.size.0 as i32, m.size.1 as i32);
    // The client's DOWN/UP/MOVE constants must be the engine's, and are 0/1/2 on the wire.
    assert_eq!((input::DOWN, input::UP, input::MOVE), (ACTION_DOWN, ACTION_UP, ACTION_MOVE));
    assert_eq!((ACTION_DOWN, ACTION_UP, ACTION_MOVE), (0, 1, 2));
    m.sends(input::touch(input::DOWN, (10, 20), (w, h)), &mirror::touch(ACTION_DOWN, 10, 20, w as u16, h as u16, 1.0));
    m.sends(input::touch(input::MOVE, (30, 40), (w, h)), &mirror::touch(ACTION_MOVE, 30, 40, w as u16, h as u16, 1.0));
    m.sends(input::touch(input::UP, (w - 1, h - 1), (w, h)), &mirror::touch(ACTION_UP, w - 1, h - 1, w as u16, h as u16, 0.0));
}

#[test]
fn secondary_and_middle_click_send_back_and_home() {
    // `MirrorView::wire_pointer`, the GestureClick: right button is Back, middle is Home.
    let mut m = mirror();
    let mut back = mirror::back_or_screen_on(ACTION_DOWN).to_vec();
    back.extend_from_slice(&mirror::back_or_screen_on(ACTION_UP));
    m.sends(input::button("back"), &back);
    m.sends(input::button("home"), &mirror::key_press(mirror::KEYCODE_HOME, META_NONE));
}

#[test]
fn wheel_and_touchpad_scroll_at_the_pointer() {
    // `MirrorView::wire_pointer`, the scroll controller: `input::scroll` with the f32 pair
    // `input::scroll_amount` returns — a wheel notch is ±1, a touchpad a fraction of one.
    let mut m = mirror();
    let (w, h) = (m.size.0 as i32, m.size.1 as i32);
    for (at, hscroll, vscroll) in [((w / 2, h / 2), 0.0_f32, -1.0_f32), ((5, 7), 0.5_f32, 0.25_f32), ((0, h - 1), -1.0_f32, 1.0_f32)] {
        m.sends(
            input::scroll(at, (w, h), hscroll, vscroll),
            &mirror::scroll(at.0, at.1, w as u16, h as u16, hscroll, vscroll),
        );
    }
}

#[test]
fn ctrl_v_pastes_the_desktop_clipboard() {
    // `MirrorView::wire_keys`, Ctrl+V: `input::paste`, one clipboard message, no `sequence`.
    let mut m = mirror();
    let text = "pasted from the desktop — ünïcode too";
    m.sends(input::paste(text), &mirror::set_clipboard(0, true, text));
}

#[test]
fn keys_go_down_and_up_with_their_meta_state() {
    // `MirrorView::wire_keys`: `input::key`, DOWN on key_pressed and UP on release, `meta` from
    // `input::meta` (shift 0x1, alt 0x2, ctrl 0x1000). Enter plain; Ctrl+Shift+Z as a chord.
    let mut m = mirror();
    for (keycode, meta) in [(66_u32, 0_u32), (54, 0x1001), (4, 0x2)] {
        m.sends(input::key(input::DOWN, keycode, meta), &mirror::keycode(ACTION_DOWN, keycode, 0, meta));
        m.sends(input::key(input::UP, keycode, meta), &mirror::keycode(ACTION_UP, keycode, 0, meta));
    }
}

#[test]
fn printable_keys_go_as_text_one_character_at_a_time() {
    // `MirrorView::wire_keys`: an unmodified printable key is `input::text(c)`.
    let mut m = mirror();
    for c in ['a', 'Z', ' ', 'é', '€'] {
        m.sends(input::text(c), &mirror::text_chunks(&c.to_string()).concat());
    }
}

#[test]
fn rail_buttons_send_their_widget_name_as_the_type() {
    // `MirrorView::new` names each rail key after its kind in `input::RAIL`, and
    // `MirrorView::wire` sends `input::button(key.widget_name())`. The kinds, in the rail's order:
    let mut m = mirror();
    let mut back = mirror::back_or_screen_on(ACTION_DOWN).to_vec();
    back.extend_from_slice(&mirror::back_or_screen_on(ACTION_UP));
    let rail: [(&str, Vec<u8>); 9] = [
        ("power", mirror::key_press(mirror::KEYCODE_POWER, META_NONE).to_vec()),
        ("volumeup", mirror::key_press(mirror::KEYCODE_VOLUME_UP, META_NONE).to_vec()),
        ("volumedown", mirror::key_press(mirror::KEYCODE_VOLUME_DOWN, META_NONE).to_vec()),
        ("back", back),
        ("home", mirror::key_press(mirror::KEYCODE_HOME, META_NONE).to_vec()),
        ("appswitch", mirror::key_press(mirror::KEYCODE_APP_SWITCH, META_NONE).to_vec()),
        ("rotate", mirror::rotate_device().to_vec()),
        ("notifications", mirror::expand_notification_panel().to_vec()),
        ("quicksettings", mirror::expand_settings_panel().to_vec()),
    ];
    // A key added to, dropped from or reordered in the client's rail fails here.
    let kinds: Vec<&str> = input::RAIL.iter().flatten().map(|(_, _, kind)| *kind).collect();
    assert_eq!(kinds, rail.iter().map(|(kind, _)| *kind).collect::<Vec<_>>());
    for (kind, expected) in rail {
        m.sends(input::button(kind), &expected);
    }
}

#[test]
fn the_display_key_turns_the_panel_off_and_on() {
    // `MirrorView::wire`, the display key: `input::display_power(on)`, toggling.
    let mut m = mirror();
    m.sends(input::display_power(false), &mirror::set_display_power(false));
    m.sends(input::display_power(true), &mirror::set_display_power(true));
}

#[test]
fn a_stalled_decoder_asks_for_resetvideo() {
    // `stream`'s send loop answers a decoder resync with `input::reset_video()`.
    let mut m = mirror();
    m.sends(input::reset_video(), &mirror::reset_video());
}
