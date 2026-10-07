//! Embedded device mirror (D59): the scrcpy wire protocol, pinned to the vendored
//! server version. Everything here is pure — arg builders, packet parsers, control
//! message encoders — so it unit-tests byte-for-byte with no device. The shell owns
//! the runtime half (adb subprocesses, TCP sockets, event pumping), same split as
//! pty handling.
//!
//! Byte layouts are transcribed from the scrcpy v4.1 sources (`app/src/control_msg.c`,
//! `app/src/demuxer.c`, `app/src/device_msg.c`, `app/src/util/binary.h`). The protocol
//! is only stable per-version — the vendored jar and [`SCRCPY_VERSION`] must move
//! together, and the version argument the server receives must match it exactly.
//!
//! Scope is deliberate, not partial. Relay implements the control messages that delete a
//! physical trip to the phone — navigation, hardware buttons, display power, rotation,
//! the notification/settings panels, and setting (and pasting) the device clipboard. It
//! does not read the clipboard back: GET_CLIPBOARD and the device→host messages (CLIPBOARD,
//! ACK_CLIPBOARD) have no consumer, so the control socket's reverse direction is drained
//! unparsed. Nor does it implement UHID create/input/destroy or the hard-keyboard settings
//! shortcut (Relay has no HID device to create; text goes over INJECT_TEXT and the
//! clipboard), START_APP
//! (`android.rs` launches through adb with an explicit activity because it needs the pid
//! back, which START_APP does not give), or the audio stream (the mirror decodes video
//! only). Their type bytes are still written down below, because the ordinals are
//! positional and a gap would shift every value after it.

use serde::{Deserialize, Serialize};

/// Must equal the vendored server's version string exactly; the server exits on
/// mismatch by design.
pub const SCRCPY_VERSION: &str = "4.1";

/// SHA-256 of the vendored `scrcpy-server-v4.1` (upstream v4.1's published hash, and the one
/// in `apps/relay-native/resources/README.md`). The jar runs on the device as the shell user,
/// so it is checked against this before every push: changing the jar takes a visible code
/// change here, not a binary diff nobody can review.
pub const SCRCPY_SERVER_SHA256: &str = "deacb991ed2509715160ffdc7907e47b4160eb30d1566217e9047fd5b8850cae";

/// Where the server jar lives on the device. Named distinctly from stock scrcpy's
/// path so Relay and a host-installed scrcpy never fight over the same file.
pub const MIRROR_SERVER_DEVICE_PATH: &str = "/data/local/tmp/scrcpy-server-relay.jar";

/// Raw codec id the server sends at video-stream start: ASCII "h264".
pub const CODEC_ID_H264: u32 = 0x6832_3634;

/// `adb push` of the vendored jar. `local_jar` is the host-side path, found and
/// hash-checked by `device::mirror_server`.
pub fn push_args(serial: &str, local_jar: &str) -> Vec<String> {
    vec![
        "-s".into(),
        serial.into(),
        "push".into(),
        // `--sync` compares size and mtime and skips the transfer itself — one small round
        // trip instead of 733,706 bytes over USB on every single mirror start, for a jar
        // that is pinned and byte-identical. Preferred over a process-local memo because
        // adb re-checks the DEVICE, so a file deleted out from under us is re-pushed
        // rather than assumed present.
        //
        // One answer changes: a device-side jar with a newer mtime than the host's would
        // be kept rather than overwritten — a Relay downgrade, since the path carries no
        // version. The failure is loud (the pinned server refuses a protocol it does not
        // know), not a silently wrong picture.
        "--sync".into(),
        local_jar.into(),
        MIRROR_SERVER_DEVICE_PATH.into(),
    ]
}

fn socket_name(scid: u32) -> String {
    format!("localabstract:scrcpy_{scid:08x}")
}

/// Forward-tunnel setup: `tcp:0` makes adb pick a free port and print it, so the
/// shell parses the port from stdout instead of racing to choose one.
pub fn forward_args(serial: &str, scid: u32) -> Vec<String> {
    vec![
        "-s".into(),
        serial.into(),
        "forward".into(),
        "tcp:0".into(),
        socket_name(scid),
    ]
}

/// Tear the tunnel down. `adb forward --remove` takes the *local* end — the `tcp:` port
/// [`forward_args`] printed — and refuses a remote socket name, so every forward removed by
/// its `localabstract:` name stayed open until the adb server restarted.
pub fn forward_remove_args(serial: &str, port: u16) -> Vec<String> {
    vec![
        "-s".into(),
        serial.into(),
        "forward".into(),
        "--remove".into(),
        format!("tcp:{port}"),
    ]
}

/// Capture rungs for `max_size` — the cap scrcpy applies to the *longer* edge of the
/// captured picture. A short fixed ladder rather than a free integer for two reasons:
/// the encoder wants dimensions that stay a multiple of 8 after scaling, and a monotone
/// ladder bounds how often a growing panel can re-negotiate the stream (at most once per
/// rung, ever). [`CAPTURE_MIN`] is D59's old hard-coded value — enough for a picture up
/// to ~1080p tall, which is exactly where it stopped scaling.
pub const CAPTURE_SIZES: [u32; 5] = [1024, 1280, 1600, 1920, 2560];
pub const CAPTURE_MIN: u32 = CAPTURE_SIZES[0];

/// Smallest rung that covers `px` device pixels of long edge (the top rung when none
/// does). Asking for more than the phone's own resolution is not an error — scrcpy only
/// ever downscales — so the top rung is safe on every device.
pub fn capture_size_for(px: u32) -> u32 {
    let top = CAPTURE_SIZES[CAPTURE_SIZES.len() - 1];
    CAPTURE_SIZES.iter().copied().find(|&r| r >= px).unwrap_or(top)
}

/// Bit rate for a rung. scrcpy's default (8 Mbps) is tuned for ~1024; held there, a
/// 2560-tall picture blocks up on motion — which would defeat capturing it large in the
/// first place. Scales with the long edge, capped: the link is local (USB/adb), so the
/// ceiling is the device encoder's comfort, not bandwidth.
pub fn capture_bit_rate(max_size: u32) -> u32 {
    let scaled = 8_000_000u64 * u64::from(capture_size_for(max_size)) / u64::from(CAPTURE_MIN);
    scaled.min(20_000_000) as u32
}

/// The stream size scrcpy will most likely pick for a `width`×`height` display capped at
/// `max_size` on the long edge: the short side scaled and rounded to a multiple of 8
/// (`(minor * max / major + 4) & !7`, the server's own rounding). Only an estimate — v4.1
/// also honours the encoder's alignment, and `wm size` reports the natural (portrait) size
/// even in landscape — so the real size always comes from the stream's session packet; this
/// is what the start response says before that packet exists.
pub fn fit_size(width: u32, height: u32, max_size: u32) -> (u32, u32) {
    let (w, h) = (width & !7, height & !7);
    let (major, minor) = (w.max(h), w.min(h));
    if major == 0 || max_size == 0 || major <= max_size {
        return (w, h);
    }
    let minor = ((u64::from(minor) * u64::from(max_size) / u64::from(major)) as u32 + 4) & !7;
    if w >= h { (max_size, minor) } else { (minor, max_size) }
}

/// The full `adb shell … app_process` invocation that boots the server on-device at one
/// capture size and bit rate — the only two values Relay varies. Everything else is a fixed
/// key with a long-stable name, since the server hard-fails on any unknown key: no audio
/// (the mirror decodes video only), stay awake while plugged in, info-level server logs (the
/// only diagnostics when it dies). Meta preludes (dummy byte, device name, codec id) and
/// frame meta stay on by default; the reader depends on them.
///
/// `max_size` is snapped to a [`CAPTURE_SIZES`] rung here, so no caller can hand the server
/// an off-ladder value. Argument order is fixed and tested: the server parses the version
/// positionally (right after the class name) and everything after it as `key=value`, so a
/// stable order is what makes the launch line something you can diff against a log.
///
/// Adding an option back: v4.1 spells an orientation lock `capture_orientation=@<angle>`;
/// the pre-3.0 `lock_video_orientation` key is unknown to it and stops it booting.
pub fn server_shell_args(serial: &str, scid: u32, max_size: u32, video_bit_rate: u32) -> Vec<String> {
    vec![
        "-s".into(),
        serial.into(),
        "shell".into(),
        format!("CLASSPATH={MIRROR_SERVER_DEVICE_PATH}"),
        "app_process".into(),
        "/".into(),
        "com.genymobile.scrcpy.Server".into(),
        SCRCPY_VERSION.into(),
        format!("scid={scid:08x}"),
        "log_level=info".into(),
        "audio=false".into(),
        "video_codec=h264".into(),
        format!("max_size={}", capture_size_for(max_size)),
        format!("video_bit_rate={video_bit_rate}"),
        "max_fps=60".into(),
        "stay_awake=true".into(),
        // Last, always: the tunnel mode is what the shell's connect sequence assumes.
        "tunnel_forward=true".into(),
    ]
}

// ---------------------------------------------------------------------------
// video stream parsing
// ---------------------------------------------------------------------------

/// One 12-byte header unit from the video socket (after the codec id).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamUnit {
    /// Video dimensions — sent at start and again on every rotation/resize.
    /// Header-only, no payload follows.
    Session { width: u32, height: u32 },
    /// A media packet header; `len` payload bytes follow. `config` packets carry
    /// SPS/PPS (no PTS semantics), `key` marks IDR frames.
    Media { config: bool, key: bool, pts: u64, len: u32 },
}

const FLAG_CONFIG: u64 = 1 << 62;
const FLAG_KEY_FRAME: u64 = 1 << 61;
const PTS_MASK: u64 = FLAG_KEY_FRAME - 1;

fn read_u32be(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Session packets are flagged by the MSB of byte 0; everything else is a media
/// header of `[u64 flags+pts][u32 payload len]`.
pub fn parse_stream_unit(header: &[u8; 12]) -> StreamUnit {
    if header[0] & 0x80 != 0 {
        return StreamUnit::Session {
            width: read_u32be(&header[4..8]),
            height: read_u32be(&header[8..12]),
        };
    }
    let word = u64::from_be_bytes(header[..8].try_into().unwrap());
    StreamUnit::Media {
        config: word & FLAG_CONFIG != 0,
        key: word & FLAG_KEY_FRAME != 0,
        pts: word & PTS_MASK,
        len: read_u32be(&header[8..12]),
    }
}

pub fn parse_codec_id(b: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*b)
}

// ---------------------------------------------------------------------------
// control messages (client -> device)
// ---------------------------------------------------------------------------

// The type byte is the ordinal of `sc_control_msg_type`, so the whole table has to be
// transcribed even for messages Relay never sends — a gap here would silently shift
// every value below it, and a wrong type byte is not an error on the wire, it is a
// different action on a real phone.
const TYPE_INJECT_KEYCODE: u8 = 0;
const TYPE_INJECT_TEXT: u8 = 1;
const TYPE_INJECT_TOUCH_EVENT: u8 = 2;
const TYPE_INJECT_SCROLL_EVENT: u8 = 3;
const TYPE_BACK_OR_SCREEN_ON: u8 = 4;
const TYPE_EXPAND_NOTIFICATION_PANEL: u8 = 5;
const TYPE_EXPAND_SETTINGS_PANEL: u8 = 6;
const TYPE_COLLAPSE_PANELS: u8 = 7;
// 8 GET_CLIPBOARD — unimplemented, see the module-level note on scope.
const TYPE_SET_CLIPBOARD: u8 = 9;
const TYPE_SET_DISPLAY_POWER: u8 = 10;
const TYPE_ROTATE_DEVICE: u8 = 11;
// 12..=14 UHID_CREATE / UHID_INPUT / UHID_DESTROY, 15 OPEN_HARD_KEYBOARD_SETTINGS,
// 16 START_APP — deliberately unimplemented, see the module-level note on scope.
const TYPE_RESET_VIDEO: u8 = 17;

/// scrcpy's fake pointer id for the mouse (-1). Using it makes the server present the
/// event as `SOURCE_MOUSE` with `TOOL_TYPE_MOUSE`, which is what makes hover, scroll and
/// button state work at all — a finger has no buttons.
pub const POINTER_ID_MOUSE: u64 = u64::MAX;
/// (-2) A synthetic finger: `SOURCE_TOUCHSCREEN`. The escape hatch for apps that
/// discriminate on tool type (games, some drawing surfaces) and for multi-touch, where
/// each simultaneous contact needs its own id.
pub const POINTER_ID_GENERIC_FINGER: u64 = u64::MAX - 1;
/// AMOTION_EVENT_BUTTON_PRIMARY.
pub const BUTTON_PRIMARY: u32 = 1;

/// Android key/motion event actions (AKEY_EVENT_ACTION_* / AMOTION_EVENT_ACTION_*).
pub const ACTION_DOWN: u8 = 0;
pub const ACTION_UP: u8 = 1;
pub const ACTION_MOVE: u8 = 2;

// -- Android keycodes (AKEYCODE_*) ------------------------------------------
//
// Named because callers otherwise write magic numbers: the frontend's key map used to
// stop at `Escape: 111`, which most apps ignore, and there was no way to say "Home"
// without knowing it is 3. These are the keys Relay's loop needs — navigation, the
// hardware buttons, and the editing keys a text field wants. Values are Android
// platform constants and do not move with the scrcpy version.

/// Home. Leaves the app under test without killing it.
pub const KEYCODE_HOME: u32 = 3;
/// Back. Prefer [`InputMsg::Back`], which routes through BACK_OR_SCREEN_ON and so also
/// wakes a sleeping screen instead of pressing Back into the void.
pub const KEYCODE_BACK: u32 = 4;
pub const KEYCODE_DPAD_UP: u32 = 19;
pub const KEYCODE_DPAD_DOWN: u32 = 20;
pub const KEYCODE_DPAD_LEFT: u32 = 21;
pub const KEYCODE_DPAD_RIGHT: u32 = 22;
pub const KEYCODE_DPAD_CENTER: u32 = 23;
pub const KEYCODE_VOLUME_UP: u32 = 24;
pub const KEYCODE_VOLUME_DOWN: u32 = 25;
/// Power. A *toggle* — it sleeps a woken screen. Use [`KEYCODE_WAKEUP`] to wake.
pub const KEYCODE_POWER: u32 = 26;
pub const KEYCODE_TAB: u32 = 61;
pub const KEYCODE_ENTER: u32 = 66;
/// Backspace (Android calls the backspace key DEL and Delete FORWARD_DEL).
pub const KEYCODE_DEL: u32 = 67;
pub const KEYCODE_MENU: u32 = 82;
pub const KEYCODE_PAGE_UP: u32 = 92;
pub const KEYCODE_PAGE_DOWN: u32 = 93;
pub const KEYCODE_ESCAPE: u32 = 111;
pub const KEYCODE_FORWARD_DEL: u32 = 112;
pub const KEYCODE_MOVE_HOME: u32 = 122;
pub const KEYCODE_MOVE_END: u32 = 123;
pub const KEYCODE_VOLUME_MUTE: u32 = 164;
/// App switch — the Recents / overview screen.
pub const KEYCODE_APP_SWITCH: u32 = 187;
/// Idempotent wake: no-op if the screen is already on, unlike [`KEYCODE_POWER`].
pub const KEYCODE_WAKEUP: u32 = 224;

// -- meta state (AMETA_*) ---------------------------------------------------

pub const META_NONE: u32 = 0;
pub const META_SHIFT_ON: u32 = 0x0000_0001;
pub const META_ALT_ON: u32 = 0x0000_0002;
pub const META_CTRL_ON: u32 = 0x0000_1000;
/// The Meta/Search key, not "meta state".
pub const META_META_ON: u32 = 0x0001_0000;

/// Server-side cap on one INJECT_TEXT message. Longer input is not an error — it must be
/// split, which [`text_chunks`] does; [`text`] keeps the single-message semantics it
/// always had and truncates.
pub const INJECT_TEXT_MAX_LENGTH: usize = 300;

/// Server-side cap on one SET_CLIPBOARD payload: the 256 KiB control-message ceiling
/// minus this message's own 14-byte header (type + u64 sequence + u8 paste + u32 len).
pub const SET_CLIPBOARD_MAX_LENGTH: usize = (1 << 18) - 14;

/// `sc_float_to_u16fp`: [0,1] -> u16 as value * 2^16, with 1.0 saturating to 0xffff.
fn float_to_u16fp(f: f32) -> u16 {
    let u = (f.clamp(0.0, 1.0) * 65536.0) as u32;
    u.min(0xffff) as u16
}

/// `sc_float_to_i16fp`: [-1,1] -> i16 as value * 2^15, with 1.0 saturating to 0x7fff.
fn float_to_i16fp(f: f32) -> i16 {
    let i = (f.clamp(-1.0, 1.0) * 32768.0) as i32;
    i.clamp(-0x8000, 0x7fff) as i16
}

/// Touch inject, fully specified: 32 bytes. Coordinates are in stream space; `w`/`h` are
/// the current stream dimensions so the device can rescale — that is what lets a recorded
/// tap survive a resolution or rotation change.
///
/// `action_button` is the button whose state *changed* in this event and `buttons` is the
/// state after it; the server keys its ACTION_BUTTON_PRESS/RELEASE synthesis off exactly
/// that pair, so they are not interchangeable. Most callers want [`touch`].
#[allow(clippy::too_many_arguments)] // it is a wire struct; naming the fields is the point
pub fn touch_pointer(
    pointer_id: u64,
    action: u8,
    x: i32,
    y: i32,
    w: u16,
    h: u16,
    pressure: f32,
    action_button: u32,
    buttons: u32,
) -> [u8; 32] {
    let mut m = [0u8; 32];
    m[0] = TYPE_INJECT_TOUCH_EVENT;
    m[1] = action;
    m[2..10].copy_from_slice(&pointer_id.to_be_bytes());
    m[10..14].copy_from_slice(&x.to_be_bytes());
    m[14..18].copy_from_slice(&y.to_be_bytes());
    m[18..20].copy_from_slice(&w.to_be_bytes());
    m[20..22].copy_from_slice(&h.to_be_bytes());
    m[22..24].copy_from_slice(&float_to_u16fp(pressure).to_be_bytes());
    m[24..28].copy_from_slice(&action_button.to_be_bytes());
    m[28..32].copy_from_slice(&buttons.to_be_bytes());
    m
}

/// Single-finger press/drag/release as the mirror's mouse produces it.
///
/// `buttons` drops to 0 on UP — that is how the server sees the press end. `action_button`
/// is PRIMARY only on the transitions (DOWN and UP): a MOVE changes no button, and
/// upstream sends 0 there. Sending PRIMARY on a MOVE claims a button-press edge that did
/// not happen, which is the kind of thing that reads fine in a log and misbehaves under a
/// view that tracks button state itself.
pub fn touch(action: u8, x: i32, y: i32, w: u16, h: u16, pressure: f32) -> [u8; 32] {
    let action_button = if action == ACTION_MOVE { 0 } else { BUTTON_PRIMARY };
    let buttons = if action == ACTION_UP { 0 } else { BUTTON_PRIMARY };
    touch_pointer(POINTER_ID_MOUSE, action, x, y, w, h, pressure, action_button, buttons)
}

/// Scroll inject: 21 bytes. `hscroll`/`vscroll` are normalized [-1,1] per tick.
pub fn scroll(x: i32, y: i32, w: u16, h: u16, hscroll: f32, vscroll: f32) -> [u8; 21] {
    let mut m = [0u8; 21];
    m[0] = TYPE_INJECT_SCROLL_EVENT;
    m[1..5].copy_from_slice(&x.to_be_bytes());
    m[5..9].copy_from_slice(&y.to_be_bytes());
    m[9..11].copy_from_slice(&w.to_be_bytes());
    m[11..13].copy_from_slice(&h.to_be_bytes());
    m[13..15].copy_from_slice(&float_to_i16fp(hscroll).to_be_bytes());
    m[15..17].copy_from_slice(&float_to_i16fp(vscroll).to_be_bytes());
    // buttons: none — trackpad-style scroll.
    m
}

/// Keycode inject: 14 bytes. `keycode` is an Android AKEYCODE_* value.
pub fn keycode(action: u8, keycode: u32, repeat: u32, meta: u32) -> [u8; 14] {
    let mut m = [0u8; 14];
    m[0] = TYPE_INJECT_KEYCODE;
    m[1] = action;
    m[2..6].copy_from_slice(&keycode.to_be_bytes());
    m[6..10].copy_from_slice(&repeat.to_be_bytes());
    m[10..14].copy_from_slice(&meta.to_be_bytes());
    m
}

/// Full key press: DOWN immediately followed by UP, 28 bytes. Android acts on the UP for
/// most navigation keys, so a lone DOWN looks like nothing happened.
pub fn key_press(kc: u32, meta: u32) -> [u8; 28] {
    let mut m = [0u8; 28];
    m[..14].copy_from_slice(&keycode(ACTION_DOWN, kc, 0, meta));
    m[14..].copy_from_slice(&keycode(ACTION_UP, kc, 0, meta));
    m
}

/// One text inject: type + u32 length + UTF-8, truncated to the server cap on a char
/// boundary. Truncating is only correct because it is one *message* — to send an
/// arbitrary string use [`text_chunks`], which splits instead of dropping.
pub fn text(s: &str) -> Vec<u8> {
    let mut end = s.len().min(INJECT_TEXT_MAX_LENGTH);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    let bytes = &s.as_bytes()[..end];
    let mut m = Vec::with_capacity(5 + bytes.len());
    m.push(TYPE_INJECT_TEXT);
    m.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    m.extend_from_slice(bytes);
    m
}

/// The whole string as one or more INJECT_TEXT messages, split on char boundaries so no
/// message exceeds the 300-byte server cap. Never truncates: the old single-message path
/// silently dropped everything past 300 bytes, which is exactly the failure you do not
/// notice until the field on the phone is missing its tail.
///
/// Splitting is per char, not per grapheme — a base char and its combining mark can land
/// in different messages. The device concatenates them into the same field, so the field
/// ends up correct; an IME watching keystrokes may see the pieces. For anything long,
/// [`set_clipboard`] with the paste flag is the better primitive anyway: one message, no
/// per-character IME work, no split at all.
pub fn text_chunks(s: &str) -> Vec<Vec<u8>> {
    if s.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut rest = s;
    while !rest.is_empty() {
        let mut end = rest.len().min(INJECT_TEXT_MAX_LENGTH);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        out.push(text(&rest[..end]));
        rest = &rest[end..];
    }
    out
}

/// BACK_OR_SCREEN_ON: 2 bytes. Presses Back when the screen is on, and wakes it when it
/// is off — the server decides, which is why this exists as its own message type rather
/// than as `keycode(KEYCODE_BACK)`. Relay's Back button uses it (via [`InputMsg::Back`])
/// precisely for that: the first press on a dark phone should wake it, not be swallowed.
pub fn back_or_screen_on(action: u8) -> [u8; 2] {
    [TYPE_BACK_OR_SCREEN_ON, action]
}

/// EXPAND_NOTIFICATION_PANEL: 1 byte. Pulls down the shade — where a crash toast, an
/// ANR dialog's notification and a foreground-service notice all live.
pub fn expand_notification_panel() -> [u8; 1] {
    [TYPE_EXPAND_NOTIFICATION_PANEL]
}

/// EXPAND_SETTINGS_PANEL: 1 byte. Quick settings — airplane mode, wifi, rotation lock,
/// the toggles a device test actually flips.
pub fn expand_settings_panel() -> [u8; 1] {
    [TYPE_EXPAND_SETTINGS_PANEL]
}

/// COLLAPSE_PANELS: 1 byte. Closes whatever the two above opened.
pub fn collapse_panels() -> [u8; 1] {
    [TYPE_COLLAPSE_PANELS]
}

/// SET_CLIPBOARD: 14-byte header + UTF-8 payload.
/// `[type][u64 sequence][u8 paste][u32 len][text]`.
///
/// `paste` asks the device to press Ctrl+V after setting the clipboard, which is the
/// correct way to get a long string into a field: one message instead of N text injects,
/// no per-character IME churn, and nothing lost to the 300-byte cap. The sequence is always
/// 0, "no ack wanted": nothing reads an ACK_CLIPBOARD back.
///
/// One message cannot be split, so text over [`SET_CLIPBOARD_MAX_LENGTH`] is cut on a char
/// boundary here; `handlers::device::send_input` refuses such text before it gets this far,
/// so a caller hears about it instead of pasting a prefix.
pub fn set_clipboard(paste: bool, s: &str) -> Vec<u8> {
    let mut end = s.len().min(SET_CLIPBOARD_MAX_LENGTH);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    let bytes = &s.as_bytes()[..end];
    let mut m = Vec::with_capacity(14 + bytes.len());
    m.push(TYPE_SET_CLIPBOARD);
    m.extend_from_slice(&0u64.to_be_bytes());
    m.push(u8::from(paste));
    m.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    m.extend_from_slice(bytes);
    m
}

/// SET_DISPLAY_POWER: 2 bytes. Turns the device's *panel* off while mirroring continues —
/// the stream keeps flowing and input keeps landing. Not a lock and not sleep: the phone
/// stays awake, it just stops lighting the room.
pub fn set_display_power(on: bool) -> [u8; 2] {
    [TYPE_SET_DISPLAY_POWER, u8::from(on)]
}

/// ROTATE_DEVICE: 1 byte. Cycles the device's own rotation, as if you had turned it —
/// the whole point being that you did not have to. The stream answers with a new
/// [`StreamUnit::Session`] carrying the new dimensions.
pub fn rotate_device() -> [u8; 1] {
    [TYPE_ROTATE_DEVICE]
}

/// RESET_VIDEO: 1 byte. Makes the server restart the video stream, so it re-sends the
/// codec config packet and a fresh IDR. The decoder cannot join a stream without
/// SPS/PPS + a key frame (D59), so a viewer that attached late or lost framing otherwise
/// has only one recovery: tear the whole mirror down and push the jar again.
pub fn reset_video() -> [u8; 1] {
    [TYPE_RESET_VIDEO]
}

// ---------------------------------------------------------------------------
// the shell's single input seam
// ---------------------------------------------------------------------------

/// Input events as the UI sends them over IPC. One enum, one encoder — the shell
/// deserializes and writes bytes, nothing more.
///
/// The named variants below (`Home`, `AppSwitch`, `Power`, …) exist so no caller has to
/// know that Recents is 187. That matters more than it sounds: the frontend is
/// JavaScript and cannot see the `KEYCODE_*` constants at all, so for it a named variant
/// *is* the constant. It is also the seam the macro recorder builds on — every variant is
/// constructible, `Clone`, and round-trips through serde in both directions, so a
/// recorded session is just a `Vec<InputMsg>` that can be stored and replayed verbatim.
///
/// Serde tag is the lowercased variant name: `{"type":"appswitch"}`, `{"type":"volumeup"}`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum InputMsg {
    Touch { action: u8, x: i32, y: i32, w: u16, h: u16, pressure: f32 },
    Scroll { x: i32, y: i32, w: u16, h: u16, hscroll: f32, vscroll: f32 },
    /// One half of a key event. For a whole press prefer `KeyPress`, which cannot be
    /// half-sent if the socket dies between the two.
    Key { action: u8, keycode: u32, #[serde(default)] meta: u32 },
    /// DOWN + UP for an arbitrary keycode, with optional modifier state.
    KeyPress { keycode: u32, #[serde(default)] meta: u32 },
    /// Every message needed to type `text` in full — chunked, never truncated.
    Text { text: String },
    /// Full press via BACK_OR_SCREEN_ON: Back if the screen is on, wake if it is off.
    Back,
    /// Home.
    Home,
    /// Recents / overview.
    AppSwitch,
    /// The hardware power button — a toggle, so it sleeps a screen that is on.
    Power,
    /// Idempotent wake (AKEYCODE_WAKEUP); does nothing to an already-lit screen.
    ScreenOn,
    VolumeUp,
    VolumeDown,
    VolumeMute,
    /// Rotate the device itself; the stream answers with new dimensions.
    Rotate,
    /// Pull down the notification shade.
    Notifications,
    /// Pull down quick settings.
    QuickSettings,
    /// Close either panel.
    Collapse,
    /// Device panel on/off while mirroring continues.
    DisplayPower { on: bool },
    /// Put `text` on the device clipboard. `paste` (default false) makes the device paste
    /// it immediately — the right way to fill a long field. At most
    /// [`SET_CLIPBOARD_MAX_LENGTH`] bytes: one message, so it cannot be split.
    SetClipboard {
        text: String,
        #[serde(default)]
        paste: bool,
    },
    /// Make the server re-send codec config + a key frame, so a stalled decoder can
    /// rejoin without restarting the mirror.
    ResetVideo,
}

pub fn encode_input(msg: &InputMsg) -> Vec<u8> {
    match msg {
        InputMsg::Touch { action, x, y, w, h, pressure } => {
            touch(*action, *x, *y, *w, *h, *pressure).to_vec()
        }
        InputMsg::Scroll { x, y, w, h, hscroll, vscroll } => {
            scroll(*x, *y, *w, *h, *hscroll, *vscroll).to_vec()
        }
        InputMsg::Key { action, keycode: kc, meta } => keycode(*action, *kc, 0, *meta).to_vec(),
        InputMsg::KeyPress { keycode: kc, meta } => key_press(*kc, *meta).to_vec(),
        // Concatenated because the caller writes one buffer: the control socket is a
        // stream, and N messages back to back are exactly N messages.
        InputMsg::Text { text: s } => text_chunks(s).concat(),
        InputMsg::Back => {
            let mut m = back_or_screen_on(ACTION_DOWN).to_vec();
            m.extend_from_slice(&back_or_screen_on(ACTION_UP));
            m
        }
        InputMsg::Home => key_press(KEYCODE_HOME, META_NONE).to_vec(),
        InputMsg::AppSwitch => key_press(KEYCODE_APP_SWITCH, META_NONE).to_vec(),
        InputMsg::Power => key_press(KEYCODE_POWER, META_NONE).to_vec(),
        InputMsg::ScreenOn => key_press(KEYCODE_WAKEUP, META_NONE).to_vec(),
        InputMsg::VolumeUp => key_press(KEYCODE_VOLUME_UP, META_NONE).to_vec(),
        InputMsg::VolumeDown => key_press(KEYCODE_VOLUME_DOWN, META_NONE).to_vec(),
        InputMsg::VolumeMute => key_press(KEYCODE_VOLUME_MUTE, META_NONE).to_vec(),
        InputMsg::Rotate => rotate_device().to_vec(),
        InputMsg::Notifications => expand_notification_panel().to_vec(),
        InputMsg::QuickSettings => expand_settings_panel().to_vec(),
        InputMsg::Collapse => collapse_panels().to_vec(),
        InputMsg::DisplayPower { on } => set_display_power(*on).to_vec(),
        InputMsg::SetClipboard { text: s, paste } => set_clipboard(*paste, s),
        InputMsg::ResetVideo => reset_video().to_vec(),
    }
}
