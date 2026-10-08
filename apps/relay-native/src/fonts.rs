//! The same font files as Relay-2, registered only with this application's map.
use gtk4::prelude::*;

/// The colour tokens every appearance mode defines, in the order of `PALETTES`' columns.
pub const TOKENS: [&str; 10] =
    ["wall", "console", "slab", "wash", "screen", "ink", "secondary", "edge", "faint", "strong"];
/// `@screen`'s column: the terminal and editor background.
pub const SCREEN: usize = 4;
/// Each appearance mode's tokens. The window's CSS (`load_appearance`) and the editor
/// schemes written below both come from this one table.
const PALETTES: [(&str, [&str; 10]); 3] = [
    ("matte", ["#0e0e10", "#141416", "#1b1b1e", "#232327", "#0a0a0b", "#ececea", "#a5a5a3", "#252529", "#77777a", "#37373c"]),
    ("dark", ["#0a0b0d", "#101114", "#16171b", "#1e1f24", "#08090a", "#eef0f2", "#a3a7ad", "#202228", "#70747b", "#33363d"]),
    ("oled", ["#000000", "#000000", "#0d0d0e", "#161618", "#000000", "#ececea", "#a5a5a3", "#1f1f22", "#77777a", "#333336"]),
];

/// The status tokens, the same in every appearance mode: theme.css defines them for the
/// window, and the editor schemes take them from here. Change both.
const STATUS: [(&str, &str); 3] = [("live", "#2ec469"), ("held", "#e5382e"), ("waiting", "#f0a828")];

/// The tokens of `mode`; an unknown mode is matte.
pub fn palette(mode: &str) -> [&'static str; 10] {
    PALETTES.iter().find(|(name, _)| *name == mode).unwrap_or(&PALETTES[0]).1
}

/// The Code editor's style scheme for one appearance mode: relay-editor.xml with each
/// `@token` replaced by that mode's colour.
fn editor_scheme(mode: &str, colors: [&str; 10]) -> String {
    let mut scheme = include_str!("../resources/relay-editor.xml").replace("RELAY_MODE", mode);
    // No token name is a prefix of another, so each `@token` replaces whole.
    for (token, color) in TOKENS.iter().zip(colors).chain(STATUS.iter().map(|(t, c)| (t, *c))) {
        scheme = scheme.replace(&format!("@{token}"), color);
    }
    scheme
}

pub fn install(window: &gtk4::ApplicationWindow) {
    let Some(map) = window.pango_context().font_map() else {
        return;
    };
    let directory = glib::user_cache_dir().join("relay-v4/fonts");
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        std::fs::create_dir_all(&directory)?;
        macro_rules! register {
            ($($name:literal),+ $(,)?) => {$(
                let bytes = include_bytes!(concat!("../resources/fonts/", $name));
                let path = directory.join($name);
                if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
                    std::fs::write(&path, bytes)?;
                }
                map.add_font_file(&path)?;
            )+};
        }
        register!(
            "fira-sans-latin-400-normal.ttf",
            "fira-sans-latin-500-normal.ttf",
            "fira-sans-latin-600-normal.ttf",
            "fira-sans-condensed-latin-500-normal.ttf",
            "fira-sans-condensed-latin-600-normal.ttf",
            "fira-sans-condensed-latin-700-normal.ttf",
            "fira-mono-latin-400-normal.ttf",
            "fira-mono-latin-500-normal.ttf",
            // The wordmark's face on the start screen (start.rs).
            "sora-latin-600-normal.ttf",
        );
        map.changed();
        window.pango_context().changed();
        window.set_font_map(Some(&map));
        let styles = directory.join("styles");
        std::fs::create_dir_all(&styles)?;
        for (mode, colors) in PALETTES {
            let scheme = editor_scheme(mode, colors);
            let path = styles.join(format!("relay-{mode}.xml"));
            if std::fs::read_to_string(&path).ok().as_deref() != Some(&scheme) {
                std::fs::write(path, scheme)?;
            }
        }
        use sourceview5::prelude::*;
        sourceview5::StyleSchemeManager::default().append_search_path(&styles.to_string_lossy());
        Ok(())
    })();
    if let Err(error) = result {
        tracing::warn!(%error, "Could not register bundled Relay fonts");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn editor_schemes_resolve_every_token() {
        for (mode, colors) in super::PALETTES {
            let scheme = super::editor_scheme(mode, colors);
            assert!(!scheme.contains('@'), "{mode}: unresolved token in relay-editor.xml");
            assert!(scheme.contains(&format!("background=\"{}\"", colors[super::SCREEN])));
        }
    }
}
