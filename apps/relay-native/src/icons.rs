//! Geometry from Relay-2's Icon.svelte, rendered by GTK without a theme substitution.
use gtk4::{self as gtk, prelude::*};

pub fn image(name: &'static str, size: i32) -> gtk::Image {
    let shapes = match name {
        "sidebar" => r#"<rect x="2" y="2" width="12" height="12"/><path d="M6 2v12"/>"#,
        "terminal" => r#"<path d="M3 4l4 4-4 4M8 12h5"/>"#,
        "code" => r#"<path d="M5 4L1.5 8 5 12M11 4l3.5 4L11 12M9.5 2.5l-3 11"/>"#,
        "board" => {
            r#"<rect x="2" y="2" width="3.5" height="12"/><rect x="6.25" y="2" width="3.5" height="9"/><rect x="10.5" y="2" width="3.5" height="6"/>"#
        }
        "notes" => r#"<path d="M3 2.5h10v11H3zM5.5 5.5h5M5.5 8h5M5.5 10.5h3.5"/>"#,
        "send" => r#"<path d="M2 8l12-5.5L9.5 14 8 9zM8 9l6-6.5"/>"#,
        "shield" => {
            r#"<path d="M8 2l5 2v3.5c0 3.3-2 5.5-5 6.8-3-1.3-5-3.5-5-6.8V4zM5.5 8l1.5 1.5 3.5-4"/>"#
        }
        "plus" => r#"<path d="M8 3v10M3 8h10"/>"#,
        "refresh" => r#"<path d="M13 8a5 5 0 1 1-1.5-3.5M13 2.5V5h-2.5"/>"#,
        "grid" => r#"<rect x="2" y="2" width="12" height="12"/><path d="M8 2v12M2 8h12"/>"#,
        "maximize" => r#"<rect x="3.5" y="3.5" width="9" height="9"/>"#,
        "pause" => r#"<path d="M5 3v10M11 3v10"/>"#,
        "play" => r#"<path d="M5 3l8 5-8 5z"/>"#,
        "resume" => r#"<path d="M3 8a5 5 0 1 0 1.5-3.5M3 2.5V5h2.5"/>"#,
        "chevron-down" => r#"<path d="M4 6l4 4 4-4"/>"#,
        "branch" => {
            r#"<circle cx="4.5" cy="3.5" r="1.5"/><circle cx="4.5" cy="12.5" r="1.5"/><circle cx="11.5" cy="5.5" r="1.5"/><path d="M4.5 5v6M11.5 7a4 4 0 0 1-4 4h-1"/>"#
        }
        _ => panic!("Unknown Relay icon: {name}"),
    };
    let image = gtk::Image::new();
    image.set_pixel_size(size);
    let paint = move |image: &gtk::Image| {
        let svg = gtk::Svg::new();
        let source = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="{}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">{shapes}</svg>"#,
            image.color()
        );
        svg.load_from_bytes(&glib::Bytes::from_owned(source.into_bytes()));
        image.set_paintable(Some(&svg));
    };
    // GTK 4.22 SVG paintables do not inherit currentColor from their widget.
    image.connect_map(paint);
    image.connect_state_flags_changed(move |image, _| paint(image));
    image
}

pub fn button(name: &'static str, caption: &str, class: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class(class);
    button.set_tooltip_text(Some(caption));
    button.update_property(&[gtk::accessible::Property::Label(caption)]);
    button.set_child(Some(&image(name, 14)));
    button.add_css_class("icon-key");
    button
}
