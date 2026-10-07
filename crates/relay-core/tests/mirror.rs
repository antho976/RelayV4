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
    // `--remove` names the local end adb printed; a remote socket name is refused.
    let rm = mirror::forward_remove_args("S", 40123);
    assert_eq!(rm, vec!["-s", "S", "forward", "--remove", "tcp:40123"]);
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
    // 300-byte cap must not split a multi-byte char. A leading ASCII byte puts every
    // é (2 bytes) on odd offsets, so the one at 299..301 straddles the cap and has to
    // be dropped whole: 1 + 149 * 2 = 299 bytes. Without the offset 300 is already a
    // boundary and a naive byte slice would pass.
    let s = format!("a{}", "é".repeat(150));
    let m = mirror::text(&s);
    let len = u32::from_be_bytes(m[1..5].try_into().unwrap()) as usize;
    assert_eq!(len, 299);
    assert_eq!(m.len(), 5 + len);
    assert_eq!(&m[5..], &s.as_bytes()[..299]);
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

    // Multi-byte chars never split: "a" + 150 * "é" = 301 bytes, and the é at 299..301
    // straddles the cap, so the split is 299 + 2 rather than 300 + 1.
    let s = format!("a{}", "é".repeat(150));
    let chunks = mirror::text_chunks(&s);
    let lens: Vec<usize> = chunks.iter().map(|c| c.len() - 5).collect();
    assert_eq!(lens, [299, 2]);
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
        capture_orientation: Some(LockOrientation::Deg90),
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
            "capture_orientation=@90",
            "power_off_on_close=true",
            "tunnel_forward=true",
        ]
    );
    // tunnel mode is always last; the shell's connect sequence assumes it.
    assert_eq!(args.last().unwrap(), "tunnel_forward=true");

    // Off-by-default keys never appear unless asked for.
    let plain = mirror::server_shell_args_with("S", 1, &MirrorOptions::default());
    assert!(!plain.iter().any(|a| a.starts_with("capture_orientation")));
    // v4.1 does not know the pre-3.0 key; sending it makes the server refuse to start.
    assert!(!args.iter().any(|a| a.starts_with("lock_video_orientation")));
    assert!(!plain.iter().any(|a| a.starts_with("power_off_on_close")));

    for (lock, want) in [
        (LockOrientation::Unlocked, "0"),
        (LockOrientation::Deg0, "@0"),
        (LockOrientation::Deg180, "@180"),
        (LockOrientation::Deg270, "@270"),
    ] {
        let o = MirrorOptions { capture_orientation: Some(lock), ..MirrorOptions::default() };
        let args = mirror::server_shell_args_with("S", 1, &o);
        assert!(args.contains(&format!("capture_orientation={want}")), "{lock:?}");
    }
}

// -- runtime: picture size, status, input fast path, worker end -------------
//
// The handoff that motivated these: a window sat on "Starting mirror…" or a frozen frame
// forever because no failure ever reached it; tap/swipe were checked against a width the
// stream did not have; and every touch-move took the store mutex.

mod runtime {
    use relay_bus::{Actor, Request};
    use relay_core::device::{MirrorRuntime, MirrorRuntimeConfig, MirrorState};
    use relay_core::mirror;
    use relay_core::{Door, Engine, Instance, Store};
    use serde_json::{json, Value};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn engine() -> Arc<Engine> {
        Engine::new(Instance::Test, Store::open_memory().unwrap())
    }

    #[allow(clippy::result_large_err)] // the bus's own error type, as `into_result` returns it
    fn call(e: &Engine, op: &str, payload: Value) -> Result<Value, relay_bus::error::BusError> {
        e.dispatch(Request::new(Actor::User, op, payload), Door::InProcess).into_result()
    }

    /// A fake adb: one device, `wm size` 1080x2400, and enough of push/forward/shell for the
    /// worker. `forward` prints `$port`; the server "process" just sleeps, because the test
    /// itself plays the server on that port. `get-state` answers from `state`.
    fn fake_adb(dir: &std::path::Path, port: u16, state: &str) -> String {
        let path = dir.join("adb");
        std::fs::write(&path, format!(r#"#!/bin/sh
[ "$1" = "-s" ] && shift 2
case "$1" in
  devices) printf 'List of devices attached\nrelay-phone device product:relay model:Pixel_9_Pro device:relay transport_id:1\n' ;;
  push) exit 0 ;;
  get-state) [ "{state}" = device ] && echo device && exit 0; echo "error: device not found" >&2; exit 1 ;;
  forward) [ "$2" = "--remove" ] && exit 0; echo {port} ;;
  shell) [ "$2" = wm ] && echo 'Physical size: 1080x2400' && exit 0; exec sleep 30 ;;
  *) exit 1 ;;
esac
"#)).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.display().to_string()
    }

    fn runtime(width: u32, height: u32) -> Arc<MirrorRuntime> {
        MirrorRuntime::new(MirrorRuntimeConfig {
            id: 1,
            device: "relay-phone".into(),
            width,
            height,
            input_width: 1080,
            input_height: 2400,
            max_size: 1024,
            bitrate: 8_000_000,
            scid: 1,
            adb: "adb".into(),
        })
    }

    /// A control socket the test can read back: what the device would receive.
    fn control_pair(runtime: &MirrorRuntime) -> TcpStream {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (device, _) = listener.accept().unwrap();
        runtime.install_control(client).unwrap();
        device.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        device
    }

    #[test]
    fn fit_size_rounds_the_short_side_to_eight_like_the_server() {
        // The old handler rounded to 2 and said 460; the server sends 464 for this phone.
        assert_eq!(mirror::fit_size(1080, 2400, 1024), (464, 1024));
        assert_eq!(mirror::fit_size(2400, 1080, 1024), (1024, 464));
        assert_eq!(mirror::fit_size(1080, 2400, 1280), (576, 1280));
        // Under the cap nothing scales, but both sides still land on a multiple of 8.
        assert_eq!(mirror::fit_size(1001, 803, 1024), (1000, 800));
        assert_eq!(mirror::fit_size(0, 0, 1024), (0, 0));
        for (w, h, max) in [(1440, 3120, 1600), (720, 1600, 1024), (1200, 1920, 1920)] {
            let (a, b) = mirror::fit_size(w, h, max);
            assert_eq!((a % 8, b % 8), (0, 0), "{w}x{h}@{max}");
            assert!(a.max(b) <= max);
        }
    }

    #[test]
    fn session_packets_move_the_size_tap_and_swipe_are_checked_against() {
        // Started from the `wm size` estimate; the device then rotates to landscape.
        let r = runtime(464, 1024);
        let mut device = control_pair(&r);
        r.set_size(1024, 464);
        assert_eq!(r.size(), (1024, 464));
        // x=900 is outside the portrait estimate, and valid in landscape.
        relay_core::handlers::device::send_input(&r, &json!({"type":"tap","x":900,"y":400})).unwrap();
        let mut bytes = [0u8; 64];
        device.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes[..32], &mirror::touch(mirror::ACTION_DOWN, 900, 400, 1024, 464, 1.0));
        assert_eq!(&bytes[32..], &mirror::touch(mirror::ACTION_UP, 900, 400, 1024, 464, 0.0));
        // y=600 was fine in portrait and is now off the picture: refused, not sent to be
        // dropped silently by the server.
        let err = relay_core::handlers::device::send_input(&r, &json!({"type":"tap","x":10,"y":600})).unwrap_err();
        assert_eq!(err.code, "device.input");
        // Status carries the new size while live, so the window can re-letterbox.
        r.set_running("Pixel".into());
        r.set_size(464, 1024);
        let status = r.status();
        assert_eq!((status.state, status.width, status.height), (MirrorState::Running, 464, 1024));
    }

    #[test]
    fn the_first_terminal_state_wins() {
        let r = runtime(464, 1024);
        assert_eq!(r.status().state, MirrorState::Starting);
        r.set_running("Pixel".into());
        assert!(r.finish(MirrorState::Lost, Some("device.mirror_device_lost".into()), Some("gone".into())));
        // A stop racing the loss must not relabel it — the window would say "Stopped" for an
        // unplugged phone.
        assert!(!r.finish(MirrorState::Stopped, None, None));
        r.set_running("again".into());
        r.set_size(1, 1);
        let status = r.status();
        assert_eq!(status.state, MirrorState::Lost);
        assert_eq!(status.code.as_deref(), Some("device.mirror_device_lost"));
        assert_eq!((status.width, status.height), (464, 1024));
        let wire = serde_json::to_value(&status).unwrap();
        assert_eq!(wire["state"], "lost");
        assert_eq!(wire["message"], "gone");
    }

    #[test]
    fn mirror_input_is_answered_without_the_store() {
        let e = engine();
        let dir = tempfile::tempdir().unwrap();
        let adb = fake_adb(dir.path(), 1, "device");
        call(&e, "settings.set", json!({"path":"device.adb_path","value":adb})).unwrap();
        let started = call(&e, "device.mirror.start", json!({"device":"relay-phone","max_size":1024})).unwrap();
        assert_eq!((started["width"].as_u64(), started["height"].as_u64()), (Some(464), Some(1024)));
        let id = started["mirror_id"].as_i64().unwrap();

        let touch = |id: i64| Request::new(
            Actor::User,
            "device.mirror.input",
            json!({"mirror_id":id,"event":{"type":"touch","action":2,"x":10,"y":10,"w":464,"h":1024,"pressure":1.0}}),
        );
        assert!(e.answers_from_memory(&touch(id)));
        assert!(!e.answers_from_memory(&touch(id + 99)));
        // Text can be big enough to block on the socket, so it keeps to the blocking pool.
        assert!(!e.answers_from_memory(&Request::new(
            Actor::User,
            "device.mirror.input",
            json!({"mirror_id":id,"event":{"type":"text","text":"hello"}}),
        )));

        // Hold the store for the whole exchange. A request that needs it would block here
        // until the guard drops; the fast path answers anyway.
        let guard = e.store.lock();
        let (done, result) = std::sync::mpsc::channel();
        let engine = e.clone();
        std::thread::spawn(move || {
            let ok = engine.dispatch(touch(id), Door::Socket).into_result();
            let missing = engine.dispatch(touch(id + 99), Door::Socket).into_result();
            let _ = done.send((ok, missing));
        });
        let (ok, missing) = result.recv_timeout(Duration::from_secs(5)).expect("mirror input waited for the store");
        drop(guard);
        ok.unwrap();
        assert_eq!(missing.unwrap_err().code, "device.mirror_not_found");
        call(&e, "device.mirror.stop", json!({"mirror_id":id})).unwrap();
    }

    /// Accept on a non-blocking listener until `stop` is set. The worker can end without ever
    /// connecting (a push or forward the fake adb does not know), and a blocking accept would
    /// then hang the test instead of failing it.
    fn accept_until(listener: &TcpListener, stop: &AtomicBool) -> Option<TcpStream> {
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    return Some(stream);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if stop.load(Ordering::SeqCst) {
                        return None;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => panic!("accept: {e}"),
            }
        }
    }

    /// Play the scrcpy server on `listener`: handshake, one session packet, a config and a key
    /// frame, then drop both sockets the way an unplugged phone does. Returns whether the
    /// worker ever connected; gives up waiting for it once `stop` is set.
    fn serve_then_vanish(listener: TcpListener, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<bool> {
        listener.set_nonblocking(true).unwrap();
        std::thread::spawn(move || {
            let Some(mut video) = accept_until(&listener, &stop) else { return false };
            video.write_all(&[0]).unwrap();
            let Some(control) = accept_until(&listener, &stop) else { return false };
            let mut name = [0u8; 64];
            name[..5].copy_from_slice(b"Pixel");
            video.write_all(&name).unwrap();
            video.write_all(b"h264").unwrap();
            let mut session = [0u8; 12];
            session[0] = 0x80;
            session[4..8].copy_from_slice(&1024u32.to_be_bytes());
            session[8..12].copy_from_slice(&464u32.to_be_bytes());
            video.write_all(&session).unwrap();
            for (flags, payload) in [(1u64 << 62, &[0, 0, 0, 1, 0x67][..]), (1u64 << 61, &[0, 0, 0, 1, 0x65][..])] {
                video.write_all(&flags.to_be_bytes()).unwrap();
                video.write_all(&(payload.len() as u32).to_be_bytes()).unwrap();
                video.write_all(payload).unwrap();
            }
            std::thread::sleep(Duration::from_millis(200));
            drop(control);
            drop(video);
            true
        })
    }

    fn run_worker_until_it_ends(get_state: &str) -> (Arc<Engine>, i64, relay_core::device::MirrorStatus, Vec<Value>) {
        let e = engine();
        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        // The source-tree jar is the server path in tests; the fake adb "pushes" it anywhere.
        assert!(relay_core::device::mirror_server_path().is_some());
        let adb = fake_adb(dir.path(), port, get_state);
        call(&e, "settings.set", json!({"path":"device.adb_path","value":adb})).unwrap();
        let mut events = e.subscribe();
        let started = call(&e, "device.mirror.start", json!({"device":"relay-phone","max_size":1024})).unwrap();
        let id = started["mirror_id"].as_i64().unwrap();
        let runtime = relay_core::handlers::device::mirror_by_id(&e, id).unwrap();
        let mut status = runtime.watch_status();
        let stop = Arc::new(AtomicBool::new(false));
        let server = serve_then_vanish(listener, stop.clone());
        let worker = {
            let (e, r) = (e.clone(), runtime.clone());
            std::thread::spawn(move || relay_core::handlers::device::mirror_worker(e, r))
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        while !status.borrow_and_update().state.is_terminal() {
            assert!(Instant::now() < deadline, "worker never reported an end: {:?}", runtime.status());
            std::thread::sleep(Duration::from_millis(20));
        }
        stop.store(true, Ordering::SeqCst);
        worker.join().unwrap();
        assert!(
            matches!(server.join(), Ok(true)),
            "worker ended before it was served: {:?}",
            runtime.status()
        );
        // The session packet set the size before "running" was announced.
        assert_eq!(runtime.size(), (1024, 464));
        let mut seen = Vec::new();
        while let Ok(event) = events.try_recv() {
            if event.ev == "mirror.changed" {
                seen.push(event.payload);
            }
        }
        (e, id, runtime.status(), seen)
    }

    #[test]
    fn a_vanished_device_ends_the_mirror_as_lost_and_says_so() {
        let (e, id, status, events) = run_worker_until_it_ends("gone");
        assert_eq!(status.state, MirrorState::Lost);
        assert_eq!(status.code.as_deref(), Some("device.mirror_device_lost"));
        assert!(status.message.unwrap().contains("relay-phone"));
        assert_eq!(status.name.as_deref(), Some("Pixel"));
        let running = events.iter().find(|ev| ev["state"] == "running").expect("running event");
        assert_eq!((running["width"].as_u64(), running["height"].as_u64()), (Some(1024), Some(464)));
        assert_eq!(running["name"], "Pixel");
        assert!(events.iter().any(|ev| ev["state"] == "lost" && ev["code"] == "device.mirror_device_lost"));
        assert!(relay_core::handlers::device::mirror_by_id(&e, id).is_err(), "an ended mirror leaves the registry");
    }

    #[test]
    fn a_dead_server_on_a_present_device_is_a_failure_with_the_reason() {
        let (_, _, status, events) = run_worker_until_it_ends("device");
        assert_eq!(status.state, MirrorState::Failed);
        assert_eq!(status.code.as_deref(), Some("device.mirror_stream_failed"));
        assert!(status.message.unwrap().contains("mirror stream ended"));
        assert!(events.iter().any(|ev| ev["state"] == "failed"));
    }

    async fn next_frame(client: &mut relay_core::socket::Client, id: i64) -> relay_bus::envelope::Frame {
        loop {
            match tokio::time::timeout(Duration::from_secs(2), client.next()).await.unwrap().unwrap().unwrap() {
                relay_core::socket::Line::Frame(frame) => {
                    assert_eq!(frame.stream, "mirror");
                    assert_eq!(frame.mirror_id, Some(id));
                    return frame;
                }
                _ => continue,
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_window_stream_carries_status_and_ends_on_a_terminal_state() {
        use relay_core::socket::{Client, SocketServer};
        let e = engine();
        let dir = tempfile::tempdir().unwrap();
        let adb = fake_adb(dir.path(), 1, "device");
        call(&e, "settings.set", json!({"path":"device.adb_path","value":adb})).unwrap();
        let server = SocketServer::start_in(e.clone(), dir.path().join("socket")).await.unwrap();
        let mut client = Client::connect(&server.path).await.unwrap();
        let started = client
            .call(&Request::new(Actor::User, "device.mirror.start", json!({"device":"relay-phone"})), |_| {})
            .await
            .unwrap()
            .into_result()
            .unwrap();
        let id = started["mirror_id"].as_i64().unwrap();
        let runtime = relay_core::handlers::device::mirror_by_id(&e, id).unwrap();
        let first = next_frame(&mut client, id).await;
        assert_eq!(first.data["state"], "starting");
        assert_eq!(first.data["width"], 464);
        runtime.set_running("Pixel".into());
        let running = next_frame(&mut client, id).await;
        assert_eq!(running.data["state"], "running");
        assert_eq!(running.data["name"], "Pixel");
        runtime.push(vec![1, 0, 0, 0, 1, 0x67]);
        assert!(next_frame(&mut client, id).await.data.is_string(), "video packets are base64 strings");
        runtime.finish(MirrorState::Lost, Some("device.mirror_device_lost".into()), Some("relay-phone is no longer connected".into()));
        let lost = next_frame(&mut client, id).await;
        assert_eq!(lost.data["state"], "lost");
        assert_eq!(lost.data["code"], "device.mirror_device_lost");
        // Nothing follows a terminal state on this mirror's stream.
        runtime.push(vec![2, 0, 0, 0, 1, 0x65]);
        assert!(tokio::time::timeout(Duration::from_millis(300), client.next()).await.is_err());
        drop(client);
    }
}
