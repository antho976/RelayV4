//! The same font files as Relay-2, registered only with this application's map.
use gtk4::prelude::*;

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
        );
        map.changed();
        window.pango_context().changed();
        window.set_font_map(Some(&map));
        let styles = directory.join("styles");
        std::fs::create_dir_all(&styles)?;
        for (mode, background) in [
            ("matte", "#0a0a0b"),
            ("dark", "#08090a"),
            ("oled", "#000000"),
        ] {
            let scheme = include_str!("../resources/relay-editor.xml")
                .replace("RELAY_MODE", mode)
                .replace("RELAY_BACKGROUND", background);
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
