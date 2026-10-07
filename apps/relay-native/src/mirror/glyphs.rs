//! The mirror's control glyphs. Same anatomy as `icons.rs` — 16-unit view box, 1.5 strokes,
//! round caps and joins, painted in the widget's own color — but kept here because they only
//! mean something next to a phone: the Android navigation triad, hardware buttons, panels.
//! Shapes the shared set already draws are delegated to `icons::image`.
use gtk4::{self as gtk, prelude::*};
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    /// Rendered glyphs by document, for the same reason as `icons.rs`: every hover, press and
    /// window focus change repaints each key, and the set of documents is small and bounded.
    static RENDERED: RefCell<HashMap<String, gtk::Svg>> = RefCell::new(HashMap::new());
}

fn geometry(name: &str) -> Option<&'static str> {
    Some(match name {
        // Android's own navigation marks, so the bar under the phone reads at a glance.
        "back" => r##"<path d="M11 3.5L4.5 8l6.5 4.5z" />"##,
        "home" => r##"<circle cx="8" cy="8" r="4.5" />"##,
        "recents" => r##"<rect x="3.75" y="3.75" width="8.5" height="8.5" rx="1" />"##,
        "power" => r##"<path d="M8 2.25v5.25" /><path d="M5 4.25a4.75 4.75 0 1 0 6 0" />"##,
        "volume-up" => r##"<path d="M2.5 6.25h2L8 3.5v9L4.5 9.75h-2z" /><path d="M10.75 8h3.5M12.5 6.25v3.5" />"##,
        "volume-down" => r##"<path d="M2.5 6.25h2L8 3.5v9L4.5 9.75h-2z" /><path d="M10.75 8h3.5" />"##,
        "rotate" => r##"<rect x="2" y="7.5" width="9" height="6" rx="1" /><path d="M5 4.5a5.5 5.5 0 0 1 8.5 3.25" /><path d="M14.25 5.5l-.75 2.25-2.25-.75" />"##,
        "notifications" => r##"<path d="M4 11V7.5A4 4 0 0 1 12 7.5V11l1.5 2h-11L4 11z" /><path d="M6.5 13.5a1.5 1.5 0 0 0 3 0" />"##,
        "quick-settings" => r##"<rect x="2" y="3" width="12" height="4" rx="2" /><rect x="2" y="9" width="12" height="4" rx="2" /><path d="M11.5 5h.01M4.5 11h.01" stroke-width="2.5" />"##,
        "screen-off" => r##"<rect x="4.25" y="1.5" width="7.5" height="13" rx="1" /><path d="M2 2l12 12" />"##,
        "screen-on" => r##"<rect x="4.25" y="1.5" width="7.5" height="13" rx="1" /><path d="M6.5 6.5h3v3h-3z" />"##,
        "screenshot" => r##"<path d="M2 5.5h2.75L6 3.75h4l1.25 1.75H14v7.25H2z" /><circle cx="8" cy="9" r="2.25" />"##,
        "dock" => r##"<rect x="2" y="2.5" width="12" height="11" /><path d="M9.5 2.5v11" />"##,
        "undock" => r##"<path d="M6 3H3v10h10v-3M9 3h4v4M13 3L7 9" />"##,
        "fullscreen" => r##"<path d="M2.5 6V2.5H6M10 2.5h3.5V6M13.5 10v3.5H10M6 13.5H2.5V10" />"##,
        "exit-fullscreen" => r##"<path d="M6 2.5V6H2.5M13.5 6H10V2.5M10 13.5V10h3.5M2.5 10H6v3.5" />"##,
        _ => return None,
    })
}

pub fn image(name: &str, size: i32) -> gtk::Image {
    let Some(shape) = geometry(name) else {
        // `device`, `close`, `check` and `chevron-down` are the shared shapes; `retry` is its `refresh`.
        return crate::icons::image(if name == "retry" { "refresh" } else { name }, size);
    };
    let image = gtk::Image::new();
    image.set_pixel_size(size);
    let paint = move |image: &gtk::Image| {
        let color = image.color();
        let document = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" color="{color}" stroke="{color}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">{shape}</svg>"##
        );
        let svg = RENDERED.with(|cache| {
            let hit = cache.borrow().get(&document).cloned();
            hit.unwrap_or_else(|| {
                let svg = gtk::Svg::from_bytes(&glib::Bytes::from_owned(document.clone().into_bytes()));
                cache.borrow_mut().insert(document, svg.clone());
                svg
            })
        });
        image.set_paintable(Some(&svg));
    };
    // GTK 4.22 SVG paintables do not inherit currentColor; repaint on state changes like icons.rs.
    image.connect_map(paint);
    image.connect_state_flags_changed(move |image, _| paint(image));
    image
}

/// An icon key with a tooltip and an accessible name — every mirror control has both.
pub fn key(name: &str, caption: &str) -> gtk::Button {
    let button = gtk::Button::new();
    // A click leaves focus on the phone: a focused key would take the next Space or Enter
    // typed for the device and press itself again. Tab still reaches it.
    button.set_focus_on_click(false);
    button.set_child(Some(&image(name, 16)));
    button.add_css_class("quiet");
    button.add_css_class("icon-key");
    button.add_css_class("mirror-key");
    button.set_tooltip_text(Some(caption));
    button.update_property(&[gtk::accessible::Property::Label(caption)]);
    button
}

/// Retarget a key made by [`key`].
pub fn rekey(button: &gtk::Button, name: &str, caption: &str) {
    button.set_child(Some(&image(name, 16)));
    button.set_tooltip_text(Some(caption));
    button.update_property(&[gtk::accessible::Property::Label(caption)]);
}
