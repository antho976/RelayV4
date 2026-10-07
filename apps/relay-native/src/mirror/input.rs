//! Key translation for the mirror. Pointer mapping, coalescing and the `device.mirror.input`
//! payloads are GTK-free and live in `relay_client::mirror::input`, re-exported here.
use gtk4::gdk;
pub use relay_client::mirror::input::*;

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
