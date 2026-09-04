//! Relay-2 Icon.svelte geometry, pinned source revision in docs/SOURCES.md.
use gtk4 as gtk;
pub fn image(name: &str, size: i32) -> gtk::Image {
    let name = match name {
        "sidebar-show-symbolic" => "sidebar",
        "system-search-symbolic" => "search",
        "view-grid-symbolic" => "layout",
        "alarm-symbolic" => "bell",
        "view-refresh-symbolic" => "refresh",
        "view-app-grid-symbolic" => "dashboard",
        "applications-science-symbolic" => "skills",
        "application-x-addon-symbolic" => "plugins",
        "accessories-text-editor-symbolic" => "notes",
        "view-list-symbolic" => "board",
        "document-edit-symbolic" => "notes",
        "text-x-generic-symbolic" => "code",
        "utilities-terminal-symbolic" => "terminal",
        "mail-unread-symbolic" => "send",
        "security-high-symbolic" => "shield",
        "window-close-symbolic" => "close",
        "window-new-symbolic" => "external",
        "edit-delete-symbolic" => "trash",
        "media-playback-pause-symbolic" => "pause",
        "media-playback-start-symbolic" => "play",
        "pan-down-symbolic" => "chevron-down",
        "view-fullscreen-symbolic" => "maximize",
        "view-more-symbolic" => "more",
        name => name,
    };
    let geometry = match name {
        "dashboard" => {
            r##"<rect x="2" y="2" width="5" height="5" /><rect x="9" y="2" width="5" height="5" /><rect x="2" y="9" width="5" height="5" /><rect x="9" y="9" width="5" height="5" />"##
        }
        "skills" => r##"<path d="M8 2v12M2 8h12M4 4l8 8M12 4l-8 8" />"##,
        "plugins" => r##"<path d="M6 2v3M10 2v3M4 5h8v3a4 4 0 0 1-8 0V5zM8 12v2" />"##,
        "bell" => {
            r##"<path d="M4 11V7.5A4 4 0 0 1 12 7.5V11l1.5 2h-11L4 11z" /><path d="M6.5 13.5a1.5 1.5 0 0 0 3 0" />"##
        }
        "search" => r##"<circle cx="7" cy="7" r="4.5" /><path d="M10.5 10.5L14 14" />"##,
        "settings" | "sliders" => {
            r##"<path d="M2 4.5h12M2 8h12M2 11.5h12" /><circle cx="5.5" cy="4.5" r="1.5" fill="#141416" /><circle cx="10.5" cy="8" r="1.5" fill="#141416" /><circle cx="6.5" cy="11.5" r="1.5" fill="#141416" />"##
        }
        "close" => r##"<path d="M4 4l8 8M12 4l-8 8" />"##,
        "minimize" => r##"<path d="M3.5 8h9" />"##,
        "maximize" => r##"<rect x="3.5" y="3.5" width="9" height="9" />"##,
        "plus" => r##"<path d="M8 3v10M3 8h10" />"##,
        "file" => r##"<path d="M4 2h5l3 3v9H4z" /><path d="M9 2v3h3" />"##,
        "folder" => r##"<path d="M2 4h4l1.5 1.5H14V13H2z" />"##,
        "folder-open" => {
            r##"<path d="M2 4h4l1.5 1.5H13V7" /><path d="M2 13l1.5-5.5H15L13.5 13z" />"##
        }
        "chevron-down" => r##"<path d="M4 6l4 4 4-4" />"##,
        "chevron-right" => r##"<path d="M6 4l4 4-4 4" />"##,
        "chevron-left" => r##"<path d="M10 4L6 8l4 4" />"##,
        "arrow-left" => r##"<path d="M13 8H3M7 4L3 8l4 4" />"##,
        "external" => r##"<path d="M6 3H3v10h10v-3M9 3h4v4M13 3L7 9" />"##,
        "branch" => {
            r##"<circle cx="4.5" cy="3.5" r="1.5" /><circle cx="4.5" cy="12.5" r="1.5" /><circle cx="11.5" cy="5.5" r="1.5" /><path d="M4.5 5v6M11.5 7a4 4 0 0 1-4 4h-1" />"##
        }
        "commit" => r##"<circle cx="8" cy="8" r="2.5" /><path d="M2 8h3.5M10.5 8H14" />"##,
        "play" => r##"<path d="M5 3l8 5-8 5z" />"##,
        "pause" => r##"<path d="M5 3v10M11 3v10" />"##,
        "resume" => r##"<path d="M3 8a5 5 0 1 0 1.5-3.5" /><path d="M3 2.5V5h2.5" />"##,
        "copy" => {
            r##"<rect x="5.5" y="5.5" width="8" height="8" /><path d="M10.5 5.5v-3h-8v8h3" />"##
        }
        "clear" => {
            r##"<path d="M9.5 2.5l4 4-6 6H4l-1.5-1.5z" /><path d="M6.5 5.5l4 4" /><path d="M7.5 12.5H14" />"##
        }
        "trash" => r##"<path d="M3 4h10M6 4V2.5h4V4M4.5 4l.7 9h5.6l.7-9" />"##,
        "check" => r##"<path d="M3 8.5l3 3 7-7" />"##,
        "undo" => r##"<path d="M3 6h7a3.5 3.5 0 0 1 0 7H7" /><path d="M5.5 3.5L3 6l2.5 2.5" />"##,
        "terminal" => r##"<path d="M3 4l4 4-4 4M8 12h5" />"##,
        "code" => r##"<path d="M5 4L1.5 8 5 12M11 4l3.5 4L11 12M9.5 2.5l-3 11" />"##,
        "board" | "columns" => {
            r##"<rect x="2" y="2" width="3.5" height="12" /><rect x="6.25" y="2" width="3.5" height="9" /><rect x="10.5" y="2" width="3.5" height="6" />"##
        }
        "modules" => r##"<path d="M8 2l6 3-6 3-6-3z" /><path d="M2 8l6 3 6-3M2 11l6 3 6-3" />"##,
        "layout" | "grid" => {
            r##"<rect x="2" y="2" width="12" height="12" /><path d="M8 2v12M2 8h12" />"##
        }
        "focus" => {
            r##"<rect x="2" y="2" width="12" height="12" /><rect x="5" y="5" width="6" height="6" />"##
        }
        "refresh" => r##"<path d="M13 8a5 5 0 1 1-1.5-3.5" /><path d="M13 2.5V5h-2.5" />"##,
        "download" => r##"<path d="M8 2v8M4.5 7L8 10.5 11.5 7M3 13h10" />"##,
        "user" => r##"<circle cx="8" cy="5.5" r="2.5" /><path d="M3 14a5 5 0 0 1 10 0" />"##,
        "cpu" => {
            r##"<rect x="4" y="4" width="8" height="8" /><rect x="6.5" y="6.5" width="3" height="3" /><path d="M6 1.5v2.5M10 1.5v2.5M6 12v2.5M10 12v2.5M1.5 6h2.5M1.5 10h2.5M12 6h2.5M12 10h2.5" />"##
        }
        "device" => {
            r##"<rect x="4.25" y="1.5" width="7.5" height="13" rx="1" /><path d="M6.5 3h3M7.25 12.5h1.5" />"##
        }
        "save" => r##"<path d="M3 3h8l2 2v8H3z" /><path d="M5 3v4h5V3M5 13V9h6v4" />"##,
        "edit" => r##"<path d="M3 13l1-3.5L11 2.5l2.5 2.5L6.5 12z" /><path d="M9.5 4l2.5 2.5" />"##,
        "merge" => {
            r##"<circle cx="4.5" cy="3.5" r="1.5" /><circle cx="4.5" cy="12.5" r="1.5" /><circle cx="11.5" cy="12.5" r="1.5" /><path d="M4.5 5v6M4.5 5a5 5 0 0 0 5 5h.5" />"##
        }
        "history" => r##"<circle cx="8" cy="8" r="5.5" /><path d="M8 5v3.5l2.5 1.5" />"##,
        "spike" => r##"<path d="M8 2v10M5 12h6M4 14h8" />"##,
        "send" => r##"<path d="M2 8l12-5.5L9.5 14 8 9z" /><path d="M8 9l6-6.5" />"##,
        "notes" => r##"<path d="M3 2.5h10v11H3z" /><path d="M5.5 5.5h5M5.5 8h5M5.5 10.5h3.5" />"##,
        "pin" => r##"<path d="M5 2.5h6M6 2.5v4l-2 2h8l-2-2v-4M8 8.5V14" />"##,
        "more" => {
            r##"<circle cx="3.5" cy="8" r=".8" fill="currentColor" stroke="none" /><circle cx="8" cy="8" r=".8" fill="currentColor" stroke="none" /><circle cx="12.5" cy="8" r=".8" fill="currentColor" stroke="none" />"##
        }
        "sidebar" => r##"<rect x="2" y="2" width="12" height="12" /><path d="M6 2v12" />"##,
        "graph" => {
            r##"<circle cx="5" cy="3" r="1.5" /><circle cx="11" cy="8" r="1.5" /><circle cx="5" cy="13" r="1.5" /><path d="M5 4.5v7M6.5 4a5 5 0 0 1 4.5 2.5M11 9.5A5 5 0 0 1 6.5 12" />"##
        }
        "shield" => {
            r##"<path d="M8 2l5 2v3.5c0 3.3-2 5.5-5 6.8-3-1.3-5-3.5-5-6.8V4z" /><path d="M5.5 8l1.5 1.5 3.5-4" />"##
        }
        _ => r##"<circle cx="8" cy="8" r="3"/>"##,
    };
    let document = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" color="#a5a5a3" stroke="#a5a5a3" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">{geometry}</svg>"##
    );
    let paint = gtk::Svg::from_bytes(&glib::Bytes::from_owned(document.into_bytes()));
    let image = gtk::Image::from_paintable(Some(&paint));
    image.set_pixel_size(size);
    image
}
