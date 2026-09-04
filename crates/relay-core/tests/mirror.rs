//! Byte-exact fixtures for the scrcpy v4.1 wire protocol (D59). Layouts mirror the
//! pinned upstream sources; if any of these fail after a server-jar bump, the
//! protocol moved and mirror.rs must be re-transcribed against the new version.

use relay_core::mirror::{
    self, parse_codec_id, parse_device_event, parse_device_msg, parse_stream_unit, CopyKey,
    DeviceEvent, DeviceMsg, InputMsg, LockOrientation, MirrorOptions, StreamUnit, ACTION_DOWN,
    ACTION_MOVE, ACTION_UP, CODEC_ID_H264, INJECT_TEXT_MAX_LENGTH, KEYCODE_APP_SWITCH,
    KEYCODE_HOME, KEYCODE_POWER, KEYCODE_VOLUME_DOWN, KEYCODE_VOLUME_UP, KEYCODE_WAKEUP,
    META_CTRL_ON, POINTER_ID_GENERIC_FINGER,
};

// -- launch args ------------------------------------------------------------

#[test]
fn server_args_pin_version_and_required_options() {
    let args = mirror::server_shell_args("SERIAL1", 0xabcd_1234, mirror::CAPTURE_MIN);
    assert_eq!(args[0], "-s");
    assert_eq!(args[1], "SERIAL1");
    assert!(args.contains(&"4.1".to_string()));
    assert!(args.contains(&"scid=abcd1234".to_string()));
    assert!(args.contains(&"tunnel_forward=true".to_string()));
    assert!(args.contains(&"audio=false".to_string()));
    assert!(args.contains(&"video_codec=h264".to_string()));
    assert!(args.iter().any(|a| a.starts_with("CLASSPATH=/data/local/tmp/")));
    // The version string sits right after the server class name — the server
    // parses positionally.
    let class = args.iter().position(|a| a == "com.genymobile.scrcpy.Server").unwrap();
    assert_eq!(args[class + 1], "4.1");
}

#[test]
fn capture_size_snaps_up_the_ladder_and_saturates() {
    // Exact rungs pass through; anything between takes the next one UP, so a panel that
    // outgrew a rung always gets more pixels than it renders, never fewer.
    assert_eq!(mirror::capture_size_for(1024), 1024);
    assert_eq!(mirror::capture_size_for(1025), 1280);
    assert_eq!(mirror::capture_size_for(1440), 1600);
    assert_eq!(mirror::capture_size_for(1920), 1920);
    // Past the top rung we ask for the top rung — never an unbounded value a device
    // encoder might refuse.
    assert_eq!(mirror::capture_size_for(4000), 2560);
    assert_eq!(mirror::capture_size_for(0), 1024);
    // Sorted, and the panel's "did the rung grow?" check depends on it staying so.
    assert!(mirror::CAPTURE_SIZES.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn bit_rate_scales_with_capture_size_and_is_capped() {
    assert_eq!(mirror::capture_bit_rate(1024), 8_000_000);
    assert_eq!(mirror::capture_bit_rate(1920), 15_000_000);
    assert_eq!(mirror::capture_bit_rate(2560), 20_000_000);
    // Off-ladder input snaps first, so the rate always matches the rung actually used.
    assert_eq!(mirror::capture_bit_rate(1100), mirror::capture_bit_rate(1280));
}

#[test]
fn server_args_carry_the_requested_capture_size() {
    let args = mirror::server_shell_args("S", 1, 1600);
    assert!(args.contains(&"max_size=1600".to_string()));
    assert!(args.contains(&"video_bit_rate=12500000".to_string()));
    // An off-ladder request can never reach the server: the builder snaps it.
    let odd = mirror::server_shell_args("S", 1, 1300);
    assert!(odd.contains(&"max_size=1600".to_string()));
}

#[test]
fn forward_args_use_ephemeral_port_and_scid_socket() {
    let args = mirror::forward_args("S", 0x00ff_0001);
    assert_eq!(args, vec!["-s", "S", "forward", "tcp:0", "localabstract:scrcpy_00ff0001"]);
    let rm = mirror::forward_remove_args("S", 0x00ff_0001);
    assert_eq!(rm, vec!["-s", "S", "forward", "--remove", "localabstract:scrcpy_00ff0001"]);
}

#[test]
fn push_args_target_relay_owned_path() {
    let args = mirror::push_args("S", "/res/scrcpy-server-v4.1");
    // Positional, so the assertion moved when `--sync` was added — which is the point of
    // asserting the whole argv instead: the jar is 733 KB over USB on every mirror start
    // unless adb is told to compare first, and losing that flag is silent.
    assert_eq!(
        args,
        vec![
            "-s",
            "S",
            "push",
            "--sync",
            "/res/scrcpy-server-v4.1",
            "/data/local/tmp/scrcpy-server-relay.jar",
        ]
    );
}

// -- stream parsing ---------------------------------------------------------

#[test]
fn codec_id_h264() {
    assert_eq!(parse_codec_id(b"h264"), CODEC_ID_H264);
}

#[test]
fn session_packet_carries_dimensions() {
    let mut h = [0u8; 12];
    h[0] = 0x80;
    h[4..8].copy_from_slice(&1080u32.to_be_bytes());
    h[8..12].copy_from_slice(&2400u32.to_be_bytes());
    assert_eq!(parse_stream_unit(&h), StreamUnit::Session { width: 1080, height: 2400 });
}

#[test]
fn media_header_flags_and_pts() {
    // config packet: bit 62, pts meaningless, len 32
    let mut h = [0u8; 12];
    h[..8].copy_from_slice(&(1u64 << 62).to_be_bytes());
    h[8..12].copy_from_slice(&32u32.to_be_bytes());
    assert_eq!(
        parse_stream_unit(&h),
        StreamUnit::Media { config: true, key: false, pts: 0, len: 32 }
    );

    // key frame with a pts: bit 61 | 123456, len 70000
    let mut h = [0u8; 12];
    h[..8].copy_from_slice(&((1u64 << 61) | 123_456).to_be_bytes());
    h[8..12].copy_from_slice(&70_000u32.to_be_bytes());
    assert_eq!(
        parse_stream_unit(&h),
        StreamUnit::Media { config: false, key: true, pts: 123_456, len: 70_000 }
    );

    // plain delta frame
    let mut h = [0u8; 12];
    h[..8].copy_from_slice(&99u64.to_be_bytes());
    h[8..12].copy_from_slice(&1500u32.to_be_bytes());
    assert_eq!(
        parse_stream_unit(&h),
        StreamUnit::Media { config: false, key: false, pts: 99, len: 1500 }
    );
}

// -- control encoders -------------------------------------------------------

#[test]
fn touch_down_is_byte_exact() {
    let m = mirror::touch(ACTION_DOWN, 100, 200, 1080, 2400, 1.0);
    #[rustfmt::skip]
    let expect: [u8; 32] = [
        2,                                  // TYPE_INJECT_TOUCH_EVENT
        0,                                  // ACTION_DOWN
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // pointer id -1 (mouse)
        0x00, 0x00, 0x00, 0x64,             // x = 100
        0x00, 0x00, 0x00, 0xc8,             // y = 200
        0x04, 0x38,                         // w = 1080
        0x09, 0x60,                         // h = 2400
        0xff, 0xff,                         // pressure 1.0 -> u16fp max
        0x00, 0x00, 0x00, 0x01,             // action_button = PRIMARY
        0x00, 0x00, 0x00, 0x01,             // buttons = PRIMARY
    ];
    assert_eq!(m, expect);
}

#[test]
fn touch_up_releases_buttons_and_pressure() {
    let m = mirror::touch(ACTION_UP, 5, 6, 1080, 2400, 0.0);
    assert_eq!(m[1], 1);
    assert_eq!(&m[22..24], &[0x00, 0x00]); // pressure 0
    assert_eq!(&m[24..28], &[0, 0, 0, 1]); // action_button stays PRIMARY
    assert_eq!(&m[28..32], &[0, 0, 0, 0]); // buttons cleared on UP
}

#[test]
fn touch_move_keeps_buttons_held_and_changes_no_button() {
    let m = mirror::touch(ACTION_MOVE, 5, 6, 1080, 2400, 1.0);
    assert_eq!(m[1], 2);
    assert_eq!(&m[28..32], &[0, 0, 0, 1]); // still held
    // action_button is the button whose state CHANGED — a drag changes none. Claiming
    // PRIMARY here fabricates a press edge the server turns into ACTION_BUTTON_PRESS.
    assert_eq!(&m[24..28], &[0, 0, 0, 0]);
}

#[test]
fn touch_pointer_allows_a_finger_and_explicit_button_state() {
    let m = mirror::touch_pointer(POINTER_ID_GENERIC_FINGER, ACTION_DOWN, 1, 2, 10, 20, 0.5, 0, 0);
    assert_eq!(&m[2..10], &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe]); // -2
    assert_eq!(&m[22..24], &[0x80, 0x00]); // 0.5 -> u16fp
    assert_eq!(&m[24..28], &[0, 0, 0, 0]);
    assert_eq!(&m[28..32], &[0, 0, 0, 0]);
    // The mouse convenience is exactly this with mouse conventions filled in.
    assert_eq!(
        mirror::touch(ACTION_DOWN, 1, 2, 10, 20, 1.0),
        mirror::touch_pointer(u64::MAX, ACTION_DOWN, 1, 2, 10, 20, 1.0, 1, 1)
    );
}

#[test]
fn scroll_is_byte_exact() {
    let m = mirror::scroll(540, 1200, 1080, 2400, 0.0, -1.0);
    #[rustfmt::skip]
    let expect: [u8; 21] = [
        3,                                  // TYPE_INJECT_SCROLL_EVENT
        0x00, 0x00, 0x02, 0x1c,             // x = 540
        0x00, 0x00, 0x04, 0xb0,             // y = 1200
        0x04, 0x38,                         // w = 1080
        0x09, 0x60,                         // h = 2400
        0x00, 0x00,                         // hscroll 0
        0x80, 0x00,                         // vscroll -1.0 -> i16fp min
        0x00, 0x00, 0x00, 0x00,             // buttons none
    ];
    assert_eq!(m, expect);
    // +1.0 saturates to 0x7fff, not 0x8000
    let m = mirror::scroll(0, 0, 1, 1, 0.0, 1.0);
    assert_eq!(&m[15..17], &[0x7f, 0xff]);
}

#[test]
fn keycode_is_byte_exact() {
    // ENTER (66) down
    let m = mirror::keycode(ACTION_DOWN, 66, 0, 0);
    #[rustfmt::skip]
    let expect: [u8; 14] = [
        0, 0,
        0x00, 0x00, 0x00, 0x42,             // keycode 66
        0x00, 0x00, 0x00, 0x00,             // repeat
        0x00, 0x00, 0x00, 0x00,             // meta
    ];
    assert_eq!(m, expect);
}

#[test]
fn text_prefixes_length_and_truncates_on_char_boundary() {
    assert_eq!(mirror::text("hi"), vec![1, 0, 0, 0, 2, b'h', b'i']);
    // 300-byte cap must not split a multi-byte char: 149 two-byte chars = 298
    // bytes, one more would land on 300 exactly; use é (2 bytes) * 151 = 302.
    let s = "é".repeat(151);
    let m = mirror::text(&s);
    let len = u32::from_be_bytes(m[1..5].try_into().unwrap()) as usize;
    assert_eq!(len, 300); // 150 chars * 2 bytes — boundary-safe
    assert_eq!(m.len(), 5 + len);
    assert!(std::str::from_utf8(&m[5..]).is_ok());
}

#[test]
fn text_chunks_never_truncate_and_stay_under_the_cap() {
    // 700 ASCII bytes = three messages (300 + 300 + 100), all of it sent.
    let s = "a".repeat(700);
    let chunks = mirror::text_chunks(&s);
    assert_eq!(chunks.len(), 3);
    let mut payload = Vec::new();
    for c in &chunks {
        assert_eq!(c[0], 1);
        let len = u32::from_be_bytes(c[1..5].try_into().unwrap()) as usize;
        assert!(len <= INJECT_TEXT_MAX_LENGTH);
        assert_eq!(c.len(), 5 + len);
        payload.extend_from_slice(&c[5..]);
    }
    assert_eq!(payload, s.as_bytes());
    assert!(mirror::text_chunks("").is_empty());

    // Multi-byte chars never split: 151 * "é" = 302 bytes -> 300 + 2.
    let s = "é".repeat(151);
    let chunks = mirror::text_chunks(&s);
    assert_eq!(chunks.len(), 2);
    for c in &chunks {
        assert!(std::str::from_utf8(&c[5..]).is_ok());
    }
    assert_eq!(
        chunks.iter().flat_map(|c| c[5..].to_vec()).collect::<Vec<_>>(),
        s.as_bytes()
    );
}

#[test]
fn encode_input_text_sends_the_whole_string() {
    // The single-message encoder truncates by design; the input seam must not.
    let s = "z".repeat(700);
    assert_eq!(mirror::text(&s).len(), 5 + 300);
    let bytes = mirror::encode_input(&InputMsg::Text { text: s.clone() });
    assert_eq!(bytes.len(), 3 * 5 + 700);
    assert_eq!(bytes, mirror::text_chunks(&s).concat());
}

// -- navigation and system keys ---------------------------------------------

#[test]
fn key_press_is_down_then_up_byte_exact() {
    let m = mirror::key_press(KEYCODE_HOME, 0);
    #[rustfmt::skip]
    let expect: [u8; 28] = [
        0, 0, 0x00, 0x00, 0x00, 0x03, 0, 0, 0, 0, 0, 0, 0, 0, // HOME down
        0, 1, 0x00, 0x00, 0x00, 0x03, 0, 0, 0, 0, 0, 0, 0, 0, // HOME up
    ];
    assert_eq!(m, expect);
    // meta rides on both halves
    let m = mirror::key_press(29, META_CTRL_ON); // Ctrl+A
    assert_eq!(&m[10..14], &[0x00, 0x00, 0x10, 0x00]);
    assert_eq!(&m[24..28], &[0x00, 0x00, 0x10, 0x00]);
}

#[test]
fn back_press_is_down_then_up() {
    // BACK_OR_SCREEN_ON, not keycode 4: the server presses Back when the screen is on
    // and wakes it when it is off.
    assert_eq!(mirror::encode_input(&InputMsg::Back), vec![4, 0, 4, 1]);
}

#[test]
fn navigation_variants_encode_the_right_keycodes() {
    let cases: &[(InputMsg, u32)] = &[
        (InputMsg::Home, KEYCODE_HOME),
        (InputMsg::AppSwitch, KEYCODE_APP_SWITCH),
        (InputMsg::Power, KEYCODE_POWER),
        (InputMsg::ScreenOn, KEYCODE_WAKEUP),
        (InputMsg::VolumeUp, KEYCODE_VOLUME_UP),
        (InputMsg::VolumeDown, KEYCODE_VOLUME_DOWN),
    ];
    for (msg, kc) in cases {
        let bytes = mirror::encode_input(msg);
        assert_eq!(bytes.len(), 28, "{msg:?}");
        assert_eq!(bytes, mirror::key_press(*kc, 0).to_vec(), "{msg:?}");
    }
    // The values themselves, pinned: these are Android platform constants and a wrong
    // one is a different button, not an error.
    assert_eq!((KEYCODE_HOME, KEYCODE_APP_SWITCH, KEYCODE_POWER), (3, 187, 26));
    assert_eq!((KEYCODE_VOLUME_UP, KEYCODE_VOLUME_DOWN, KEYCODE_WAKEUP), (24, 25, 224));
    assert_eq!(mirror::encode_input(&InputMsg::KeyPress { keycode: 66, meta: 0 }).len(), 28);
}

#[test]
fn panel_rotate_power_and_reset_are_byte_exact() {
    assert_eq!(mirror::expand_notification_panel(), [5]);
    assert_eq!(mirror::expand_settings_panel(), [6]);
    assert_eq!(mirror::collapse_panels(), [7]);
    assert_eq!(mirror::set_display_power(true), [10, 1]);
    assert_eq!(mirror::set_display_power(false), [10, 0]);
    assert_eq!(mirror::rotate_device(), [11]);
    assert_eq!(mirror::reset_video(), [17]);

    assert_eq!(mirror::encode_input(&InputMsg::Notifications), vec![5]);
    assert_eq!(mirror::encode_input(&InputMsg::QuickSettings), vec![6]);
    assert_eq!(mirror::encode_input(&InputMsg::Collapse), vec![7]);
    assert_eq!(mirror::encode_input(&InputMsg::DisplayPower { on: false }), vec![10, 0]);
    assert_eq!(mirror::encode_input(&InputMsg::Rotate), vec![11]);
    assert_eq!(mirror::encode_input(&InputMsg::ResetVideo), vec![17]);
}

// -- clipboard --------------------------------------------------------------

#[test]
fn get_clipboard_is_byte_exact() {
    assert_eq!(mirror::get_clipboard(CopyKey::None), [8, 0]);
    assert_eq!(mirror::get_clipboard(CopyKey::Copy), [8, 1]);
    assert_eq!(mirror::get_clipboard(CopyKey::Cut), [8, 2]);
    assert_eq!(mirror::encode_input(&InputMsg::GetClipboard { copy: CopyKey::Copy }), vec![8, 1]);
}

#[test]
fn set_clipboard_is_byte_exact() {
    let m = mirror::set_clipboard(0x0102_0304_0506_0708, true, "hi");
    #[rustfmt::skip]
    let expect: Vec<u8> = vec![
        9,                                              // TYPE_SET_CLIPBOARD
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, // sequence
        1,                                              // paste
        0x00, 0x00, 0x00, 0x02,                         // length
        b'h', b'i',
    ];
    assert_eq!(m, expect);
    // sequence 0 = no ack wanted, paste off
    assert_eq!(mirror::set_clipboard(0, false, ""), vec![9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    let msg = InputMsg::SetClipboard { text: "hi".into(), paste: true, sequence: 7 };
    assert_eq!(mirror::encode_input(&msg), mirror::set_clipboard(7, true, "hi"));
}

#[test]
fn set_clipboard_caps_on_a_char_boundary() {
    // 256 KiB minus the 14-byte header. A multi-byte char straddling the cap must be
    // dropped whole, or the device decodes garbage at the tail.
    let max = (1 << 18) - 14;
    assert_eq!(mirror::SET_CLIPBOARD_MAX_LENGTH, max);
    // "€" is 3 bytes and does not divide the cap: 87376 * 3 = 262128, and the char that
    // would end at 262131 has to be dropped whole or the device decodes garbage.
    let s = "€".repeat(max); // three times the cap in bytes
    let m = mirror::set_clipboard(1, false, &s);
    let len = u32::from_be_bytes(m[10..14].try_into().unwrap()) as usize;
    assert_eq!(len, 262_128);
    assert_eq!(m.len(), 14 + len);
    assert!(std::str::from_utf8(&m[14..]).is_ok());
}

#[test]
fn encode_input_matches_direct_encoders() {
    let touch = InputMsg::Touch { action: 0, x: 1, y: 2, w: 10, h: 20, pressure: 1.0 };
    assert_eq!(mirror::encode_input(&touch), mirror::touch(0, 1, 2, 10, 20, 1.0).to_vec());
    let key = InputMsg::Key { action: 1, keycode: 4, meta: 0 };
    assert_eq!(mirror::encode_input(&key), mirror::keycode(1, 4, 0, 0).to_vec());
}

#[test]
fn input_msg_deserializes_from_ui_json() {
    let m: InputMsg = serde_json::from_str(
        r#"{"type":"touch","action":0,"x":10,"y":20,"w":1080,"h":2400,"pressure":1.0}"#,
    )
    .unwrap();
    assert_eq!(m, InputMsg::Touch { action: 0, x: 10, y: 20, w: 1080, h: 2400, pressure: 1.0 });
    // `meta` is optional on key events
    let m: InputMsg = serde_json::from_str(r#"{"type":"key","action":0,"keycode":66}"#).unwrap();
    assert_eq!(m, InputMsg::Key { action: 0, keycode: 66, meta: 0 });
    let m: InputMsg = serde_json::from_str(r#"{"type":"back"}"#).unwrap();
    assert_eq!(m, InputMsg::Back);
    // navigation variants are plain tags
    let m: InputMsg = serde_json::from_str(r#"{"type":"appswitch"}"#).unwrap();
    assert_eq!(m, InputMsg::AppSwitch);
    let m: InputMsg = serde_json::from_str(r#"{"type":"displaypower","on":false}"#).unwrap();
    assert_eq!(m, InputMsg::DisplayPower { on: false });
    // clipboard flags default off
    let m: InputMsg = serde_json::from_str(r#"{"type":"setclipboard","text":"x"}"#).unwrap();
    assert_eq!(m, InputMsg::SetClipboard { text: "x".into(), paste: false, sequence: 0 });
    let m: InputMsg = serde_json::from_str(r#"{"type":"getclipboard"}"#).unwrap();
    assert_eq!(m, InputMsg::GetClipboard { copy: CopyKey::None });
    let m: InputMsg = serde_json::from_str(r#"{"type":"getclipboard","copy":"cut"}"#).unwrap();
    assert_eq!(m, InputMsg::GetClipboard { copy: CopyKey::Cut });
}

#[test]
fn input_msg_round_trips_for_the_macro_recorder() {
    // A recorder builds these from outside the module, stores them and replays them, so
    // every variant must survive serialize -> deserialize unchanged and be Clone.
    let recording = vec![
        InputMsg::Touch { action: 0, x: 1, y: 2, w: 1080, h: 2400, pressure: 1.0 },
        InputMsg::Touch { action: 2, x: 3, y: 4, w: 1080, h: 2400, pressure: 1.0 },
        InputMsg::Touch { action: 1, x: 3, y: 4, w: 1080, h: 2400, pressure: 0.0 },
        InputMsg::Scroll { x: 5, y: 6, w: 1080, h: 2400, hscroll: 0.0, vscroll: -1.0 },
        InputMsg::Key { action: 0, keycode: 66, meta: 0 },
        InputMsg::KeyPress { keycode: 66, meta: META_CTRL_ON },
        InputMsg::Text { text: "hello".into() },
        InputMsg::Back,
        InputMsg::Home,
        InputMsg::AppSwitch,
        InputMsg::Power,
        InputMsg::ScreenOn,
        InputMsg::VolumeUp,
        InputMsg::VolumeDown,
        InputMsg::VolumeMute,
        InputMsg::Rotate,
        InputMsg::Notifications,
        InputMsg::QuickSettings,
        InputMsg::Collapse,
        InputMsg::DisplayPower { on: true },
        InputMsg::SetClipboard { text: "long".into(), paste: true, sequence: 9 },
        InputMsg::GetClipboard { copy: CopyKey::Copy },
        InputMsg::ResetVideo,
    ];
    let json = serde_json::to_string(&recording.clone()).unwrap();
    let back: Vec<InputMsg> = serde_json::from_str(&json).unwrap();
    assert_eq!(back, recording);
    // and every one of them encodes to something
    for m in &back {
        assert!(!mirror::encode_input(m).is_empty(), "{m:?} encoded to nothing");
    }
}

// -- device message drain ---------------------------------------------------

#[test]
fn device_msg_framing() {
    // clipboard: type 0 + u32 len + text
    let mut msg = vec![0u8, 0, 0, 0, 3];
    msg.extend_from_slice(b"abc");
    assert_eq!(parse_device_msg(&msg), DeviceMsg::Skip(8));
    assert_eq!(parse_device_msg(&msg[..6]), DeviceMsg::NeedMore);
    assert_eq!(parse_device_msg(&msg[..3]), DeviceMsg::NeedMore);
    assert_eq!(parse_device_msg(&[]), DeviceMsg::NeedMore);

    // ack clipboard: fixed 9 bytes
    let ack = [1u8, 0, 0, 0, 0, 0, 0, 0, 7];
    assert_eq!(parse_device_msg(&ack), DeviceMsg::Skip(9));
    assert_eq!(parse_device_msg(&ack[..8]), DeviceMsg::NeedMore);

    // uhid output: type 2 + u16 id + u16 size + data
    let uhid = [2u8, 0, 1, 0, 2, 0xaa, 0xbb];
    assert_eq!(parse_device_msg(&uhid), DeviceMsg::Skip(7));
    assert_eq!(parse_device_msg(&uhid[..5]), DeviceMsg::NeedMore);

    assert_eq!(parse_device_msg(&[9u8, 1, 2]), DeviceMsg::Unknown(9));
}

#[test]
fn device_events_decode_content_and_agree_with_the_framing_parser() {
    let mut clip = vec![0u8, 0, 0, 0, 3];
    clip.extend_from_slice("héllo".as_bytes()); // longer than 3 — trailing bytes are the next message
    assert_eq!(
        parse_device_event(&clip[..8]),
        DeviceEvent::Clipboard { consumed: 8, text: "hé".into() }
    );

    let mut clip = vec![0u8, 0, 0, 0, 6];
    clip.extend_from_slice("héllo".as_bytes());
    assert_eq!(
        parse_device_event(&clip),
        DeviceEvent::Clipboard { consumed: 11, text: "héllo".into() }
    );
    assert_eq!(parse_device_event(&clip[..7]), DeviceEvent::NeedMore);
    assert_eq!(parse_device_event(&clip[..2]), DeviceEvent::NeedMore);
    assert_eq!(parse_device_event(&[]), DeviceEvent::NeedMore);

    // invalid utf-8 must not stall the socket — lossy, framed, consumed
    let bad = [0u8, 0, 0, 0, 2, 0xff, 0xfe];
    assert_eq!(
        parse_device_event(&bad),
        DeviceEvent::Clipboard { consumed: 7, text: "\u{fffd}\u{fffd}".into() }
    );

    let ack = [1u8, 0, 0, 0, 0, 0, 0, 0, 7];
    assert_eq!(parse_device_event(&ack), DeviceEvent::ClipboardAck { consumed: 9, sequence: 7 });
    assert_eq!(parse_device_event(&ack[..8]), DeviceEvent::NeedMore);

    let uhid = [2u8, 0, 1, 0, 2, 0xaa, 0xbb];
    assert_eq!(
        parse_device_event(&uhid),
        DeviceEvent::UhidOutput { consumed: 7, id: 1, data: vec![0xaa, 0xbb] }
    );
    assert_eq!(parse_device_event(&[9u8, 1, 2]), DeviceEvent::Unknown(9));

    // Both parsers frame identically — that is the whole reason they share a length fn.
    for buf in [&clip[..], &ack[..], &uhid[..], &bad[..], &[9u8, 1][..], &[][..]] {
        let framed = match parse_device_msg(buf) {
            DeviceMsg::Skip(n) => Some(n),
            _ => None,
        };
        let evented = match parse_device_event(buf) {
            DeviceEvent::Clipboard { consumed, .. }
            | DeviceEvent::ClipboardAck { consumed, .. }
            | DeviceEvent::UhidOutput { consumed, .. } => Some(consumed),
            _ => None,
        };
        assert_eq!(framed, evented, "{buf:?}");
    }
}

#[test]
fn clipboard_set_then_ack_round_trips_the_sequence() {
    let sent = mirror::set_clipboard(0xdead_beef, true, "paste me");
    let seq = u64::from_be_bytes(sent[1..9].try_into().unwrap());
    let mut ack = vec![1u8];
    ack.extend_from_slice(&seq.to_be_bytes());
    assert_eq!(parse_device_event(&ack), DeviceEvent::ClipboardAck { consumed: 9, sequence: seq });
}

// -- server option builder --------------------------------------------------

#[test]
fn default_options_reproduce_the_pinned_launch_line() {
    // Switching a call site to the builder must not change one byte of what the vendored
    // jar is asked to do — it hard-fails on an unknown key.
    // At the bottom capture rung the two agree by construction: `for_capture(CAPTURE_MIN)`
    // is `default()`, which is what keeps the historical line pinned through the ladder.
    let opts = MirrorOptions::default();
    assert_eq!(opts, MirrorOptions::for_capture(mirror::CAPTURE_MIN));
    assert_eq!(
        mirror::server_shell_args_with("S", 0xabcd_1234, &opts),
        mirror::server_shell_args("S", 0xabcd_1234, mirror::CAPTURE_MIN)
    );
    assert_eq!(
        mirror::server_shell_args("S", 0xabcd_1234, mirror::CAPTURE_MIN),
        vec![
            "-s",
            "S",
            "shell",
            "CLASSPATH=/data/local/tmp/scrcpy-server-relay.jar",
            "app_process",
            "/",
            "com.genymobile.scrcpy.Server",
            "4.1",
            "scid=abcd1234",
            "log_level=info",
            "audio=false",
            "video_codec=h264",
            "max_size=1024",
            "video_bit_rate=8000000",
            "max_fps=60",
            "stay_awake=true",
            "tunnel_forward=true",
        ]
    );
}

#[test]
fn options_emit_in_a_fixed_order_and_omit_server_defaults() {
    let opts = MirrorOptions {
        max_size: 720,
        video_bit_rate: 2_000_000,
        max_fps: 30,
        lock_video_orientation: Some(LockOrientation::Deg90),
        audio: true,
        stay_awake: false,
        power_off_on_close: true,
        log_level: "debug".into(),
    };
    let args = mirror::server_shell_args_with("S", 1, &opts);
    assert_eq!(
        &args[7..],
        &[
            "4.1",
            "scid=00000001",
            "log_level=debug",
            "audio=true",
            "video_codec=h264",
            "max_size=720",
            "video_bit_rate=2000000",
            "max_fps=30",
            "stay_awake=false",
            "lock_video_orientation=90",
            "power_off_on_close=true",
            "tunnel_forward=true",
        ]
    );
    // tunnel mode is always last; the shell's connect sequence assumes it.
    assert_eq!(args.last().unwrap(), "tunnel_forward=true");

    // Off-by-default keys never appear unless asked for.
    let plain = mirror::server_shell_args_with("S", 1, &MirrorOptions::default());
    assert!(!plain.iter().any(|a| a.starts_with("lock_video_orientation")));
    assert!(!plain.iter().any(|a| a.starts_with("power_off_on_close")));

    for (lock, want) in [
        (LockOrientation::Unlocked, "unlocked"),
        (LockOrientation::Deg0, "0"),
        (LockOrientation::Deg180, "180"),
        (LockOrientation::Deg270, "270"),
    ] {
        let o = MirrorOptions { lock_video_orientation: Some(lock), ..MirrorOptions::default() };
        let args = mirror::server_shell_args_with("S", 1, &o);
        assert!(args.contains(&format!("lock_video_orientation={want}")), "{lock:?}");
    }
}
