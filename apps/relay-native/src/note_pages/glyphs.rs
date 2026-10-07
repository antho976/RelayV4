//! Editor glyphs the shared icon set does not carry, drawn on the same 16px grid and
//! painted by the same painter and cache (`icons::from_geometry`). Shared names are delegated
//! to `icons::image`.
use gtk4 as gtk;

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
        "sort" => r##"<path d="M4.5 3v10M2.5 11l2 2 2-2" /><path d="M8.5 4h5.5M8.5 8h4M8.5 12h2.5" />"##,
        // Rounded, finer redraws of the shared sidebar and save glyphs, for the 1.25 stroke the
        // toolbar draws with; the shared set keeps its own for the main window.
        "note-new" => {
            r##"<path d="M9.25 2.5H5a1.5 1.5 0 0 0-1.5 1.5v8A1.5 1.5 0 0 0 5 13.5h6a1.5 1.5 0 0 0 1.5-1.5V5.75z" /><path d="M9.25 2.5v2.25a1 1 0 0 0 1 1h2.25" /><path d="M8 7.5v3.5M6.25 9.25h3.5" />"##
        }
        "sidebar" => {
            r##"<rect x="2.5" y="3" width="11" height="10" rx="1.75" /><path d="M6.25 3v10" /><path d="M4.1 5.5h.6M4.1 7.25h.6" />"##
        }
        "save" => {
            r##"<path d="M3.5 4.25c0-.97.78-1.75 1.75-1.75h5l2.25 2.25v7c0 .97-.78 1.75-1.75 1.75h-5.5c-.97 0-1.75-.78-1.75-1.75z" /><path d="M5.75 2.75v2.5h3.75v-2.5" /><path d="M5.5 13.25v-3a.75.75 0 0 1 .75-.75h3.5a.75.75 0 0 1 .75.75v3" />"##
        }
        "image" => {
            r##"<rect x="2.5" y="3" width="11" height="10" rx="1.75" /><circle cx="6" cy="6.5" r="1.1" /><path d="M3 11.25l2.75-2.75 2.25 2.25 2.25-2.25 2.75 2.75" />"##
        }
        "task" => r##"<rect x="2.5" y="2.5" width="11" height="11" rx="2" /><path d="M5.25 8.25 7 10l3.75-4" />"##,
        _ => return None,
    })
}

pub fn glyph(name: &str, size: i32) -> gtk::Image {
    glyph_stroke(name, size, 1.5)
}

/// `glyph` at another stroke: the toolbar draws 1.25 for a finer line at 16px.
pub fn glyph_stroke(name: &str, size: i32, stroke: f64) -> gtk::Image {
    let Some(geometry) = geometry(name) else {
        return crate::icons::image_with_stroke(name, size, stroke);
    };
    crate::icons::from_geometry(geometry, size, stroke)
}
