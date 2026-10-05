//! Editor glyphs the shared icon set does not carry, drawn on the same 16px grid and
//! painted the same way (see `icons.rs`). Shared names are delegated to `icons::image`.
use gtk4::{self as gtk, prelude::*};
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static RENDERED: RefCell<HashMap<String, gtk::Svg>> = RefCell::new(HashMap::new());
}

fn geometry(name: &str) -> Option<&'static str> {
    Some(match name {
        "redo" => r##"<path d="M13 6H6a3.5 3.5 0 0 0 0 7h3" /><path d="M10.5 3.5L13 6l-2.5 2.5" />"##,
        "cut" => {
            r##"<circle cx="4.5" cy="11.5" r="2" /><circle cx="11.5" cy="11.5" r="2" /><path d="M5.8 10L11.5 2.5M10.2 10L4.5 2.5" />"##
        }
        "paste" => {
            r##"<path d="M5.5 3H3.5v11h9V3h-2" /><rect x="5.5" y="1.75" width="5" height="2.5" /><path d="M6 8h4M6 10.5h4" />"##
        }
        "replace" => {
            r##"<path d="M2.5 5.5h8M8 3l2.5 2.5L8 8" /><path d="M13.5 10.5h-8M8 8l-2.5 2.5L8 13" />"##
        }
        "chevron-up" => r##"<path d="M4 10l4-4 4 4" />"##,
        "list" => {
            r##"<path d="M6 4h8M6 8h8M6 12h8" /><circle cx="2.75" cy="4" r=".75" fill="currentColor" stroke="none" /><circle cx="2.75" cy="8" r=".75" fill="currentColor" stroke="none" /><circle cx="2.75" cy="12" r=".75" fill="currentColor" stroke="none" />"##
        }
        "list-numbered" => {
            r##"<path d="M6.5 4h7.5M6.5 8h7.5M6.5 12h7.5" /><path d="M2 3l1.25-.75V5.5" stroke-width="1.1" /><path d="M1.75 10.25a1 1 0 0 1 2 .2c0 .8-2 1.6-2 2.3h2.1" stroke-width="1.1" />"##
        }
        "checklist" => {
            r##"<rect x="2" y="2.5" width="4" height="4" /><path d="M2.5 11.5l1.25 1.25 2.25-2.75" /><path d="M8.5 4.5H14M8.5 11.5H14" />"##
        }
        "link" => {
            r##"<path d="M6.5 9.5l3-3" /><path d="M7.25 4.75l1.5-1.5a2.47 2.47 0 0 1 3.5 3.5l-1.5 1.5" /><path d="M8.75 11.25l-1.5 1.5a2.47 2.47 0 0 1-3.5-3.5l1.5-1.5" />"##
        }
        "quote" => r##"<path d="M3 3.5v9" /><path d="M6.5 5h7M6.5 8h7M6.5 11h4.5" />"##,
        "code-block" => {
            r##"<rect x="2" y="2.5" width="12" height="11" /><path d="M6.5 6L4.5 8l2 2M9.5 6l2 2-2 2" />"##
        }
        "code-inline" => r##"<path d="M5.5 4.5L2 8l3.5 3.5M10.5 4.5L14 8l-3.5 3.5" />"##,
        "rule" => r##"<path d="M2 8h12" /><path d="M5 4.5h6M5 11.5h6" stroke-opacity=".45" />"##,
        "sort" => r##"<path d="M4.5 3v10M2.5 11l2 2 2-2" /><path d="M8.5 4h5.5M8.5 8h4M8.5 12h2.5" />"##,
        "note-new" => {
            r##"<path d="M4 2h5l3 3v9H4z" /><path d="M9 2v3h3" /><path d="M8 7.5v4.5M5.75 9.75h4.5" />"##
        }
        "calendar" => {
            r##"<rect x="2.5" y="3.5" width="11" height="10" /><path d="M2.5 6.5h11M5.5 2v3M10.5 2v3" />"##
        }
        "goto" => r##"<path d="M3 8h9M8.5 4.5L12 8l-3.5 3.5" /><path d="M14 3v10" />"##,
        _ => return None,
    })
}

pub fn glyph(name: &str, size: i32) -> gtk::Image {
    let Some(geometry) = geometry(name) else {
        return crate::icons::image(name, size);
    };
    let image = gtk::Image::new();
    image.set_pixel_size(size);
    let paint = move |image: &gtk::Image| {
        let color = image.color();
        let document = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" color="{color}" stroke="{color}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">{geometry}</svg>"##
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
    // GTK 4.22 SVG paintables do not inherit currentColor from their widget.
    image.connect_map(paint);
    image.connect_state_flags_changed(move |image, _| paint(image));
    image
}
