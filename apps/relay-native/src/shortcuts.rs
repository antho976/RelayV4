use gtk4::gdk::{Key, ModifierType};

pub const DEFAULTS: [(&str, &str, &str); 7] = [
    ("palette", "Command palette", "Ctrl+K"),
    ("agents", "Agents", "Ctrl+1"),
    ("code", "Code", "Ctrl+2"),
    ("board", "Board", "Ctrl+3"),
    ("new_session", "New session", "Ctrl+N"),
    ("settings", "Settings", "Ctrl+,"),
    ("sidebar", "Toggle sidebar", "Ctrl+Shift+B"),
];

fn parse(chord: &str) -> Option<(ModifierType, char)> {
    let mut mods = ModifierType::empty();
    let mut key = None;
    for part in chord.split('+').map(str::trim) {
        match part.to_lowercase().as_str() {
            "ctrl" => mods |= ModifierType::CONTROL_MASK,
            "meta" => mods |= ModifierType::META_MASK,
            "alt" => mods |= ModifierType::ALT_MASK,
            "shift" => mods |= ModifierType::SHIFT_MASK,
            s if s.chars().count() == 1 && key.is_none() => key = s.chars().next(),
            _ => return None,
        }
    }
    Some((mods, key?))
}

pub fn valid(chord: &str) -> bool {
    parse(chord).is_some_and(|(mods, _)| {
        mods.intersects(
            ModifierType::CONTROL_MASK | ModifierType::META_MASK | ModifierType::ALT_MASK,
        )
    })
}

/// Ctrl+letter is a control character a terminal program reads (Ctrl+K kills to the end of
/// the line, Ctrl+N is next history), so a focused agent terminal keeps it.
pub fn terminal_owns(chord: &str) -> bool {
    parse(chord).is_some_and(|(mods, key)| mods == ModifierType::CONTROL_MASK && key.is_ascii_alphabetic())
}

pub fn matches(key: Key, mods: ModifierType, chord: &str) -> bool {
    let mask = ModifierType::CONTROL_MASK
        | ModifierType::META_MASK
        | ModifierType::ALT_MASK
        | ModifierType::SHIFT_MASK;
    parse(chord).is_some_and(|(expected, letter)| {
        mods & mask == expected
            && key.to_unicode().and_then(|c| c.to_lowercase().next()) == Some(letter)
    })
}

#[test]
fn chords_preserve_modifiers_and_allow_disabling() {
    assert!(matches(Key::K, ModifierType::CONTROL_MASK, " ctrl + k "));
    assert!(!matches(
        Key::K,
        ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK,
        "Ctrl+K"
    ));
    assert!(matches(Key::comma, ModifierType::CONTROL_MASK, "Ctrl+,"));
    assert!(!matches(Key::a, ModifierType::empty(), ""));
    assert!(!valid("K"));
    assert!(!valid("Ctrl+Foo"));
    assert!(!valid("Ctrl+K+L"));
    assert!(terminal_owns("Ctrl+K"));
    assert!(!terminal_owns("Ctrl+1"));
    assert!(!terminal_owns("Ctrl+Shift+B"));
}
