//! Relay-2 Icon.svelte geometry, pinned source revision in docs/SOURCES.md.
use gtk4::{self as gtk, prelude::*};
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    /// Rendered icons, by their exact SVG document.
    ///
    /// Icons repaint on map *and on every state change* — hover, focus, pressed, insensitive —
    /// and the shell has dozens of them, several per session row. Re-parsing the same handful
    /// of documents on each of those was work the toolkit did not need to see; the set is tiny
    /// and bounded by the pinned geometry above, so it is simply kept.
    static RENDERED: RefCell<HashMap<String, gtk::Svg>> = RefCell::new(HashMap::new());
}

fn rendered(document: String) -> gtk::Svg {
    RENDERED.with(|cache| {
        let hit = cache.borrow().get(&document).cloned();
        if let Some(svg) = hit {
            return svg;
        }
        let svg = gtk::Svg::from_bytes(&glib::Bytes::from_owned(document.clone().into_bytes()));
        cache.borrow_mut().insert(document, svg.clone());
        svg
    })
}

pub fn image(name: &str, size: i32) -> gtk::Image {
    image_with_stroke(name, size, 1.5)
}

pub fn image_with_stroke(name: &str, size: i32, stroke: f64) -> gtk::Image {
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
        "skills" => r##"<path d="M9 1.5L3.5 9h4L7 14.5 12.5 7h-4z" />"##,
        "plugins" => {
            r##"<rect x="2" y="8" width="6" height="6" /><rect x="8" y="8" width="6" height="6" /><rect x="2" y="2" width="6" height="6" /><rect x="10" y="2" width="4" height="4" />"##
        }
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
        // File-type glyphs for the explorer and the Git panel; the tint comes from a CSS class.
        "file-rust" => {
            r##"<circle cx="8" cy="8" r="3.75" /><circle cx="8" cy="8" r="1.25" /><path d="M8 1.75v2.5M8 11.75v2.5M1.75 8h2.5M11.75 8h2.5M3.6 3.6l1.75 1.75M10.65 10.65l1.75 1.75M3.6 12.4l1.75-1.75M10.65 5.35l1.75-1.75" />"##
        }
        "file-ts" => {
            r##"<rect x="2.25" y="2.25" width="11.5" height="11.5" rx="1.5" /><path d="M4.75 7h3.5M6.5 7v4.5M12 7.5a1.4 1.4 0 0 0-1.3-.5c-.8 0-1.3.4-1.3 1s.5.9 1.3 1.1 1.3.5 1.3 1.1-.5 1.05-1.3 1.05a1.6 1.6 0 0 1-1.45-.65" />"##
        }
        "file-js" => {
            r##"<rect x="2.25" y="2.25" width="11.5" height="11.5" rx="1.5" /><path d="M7.25 6.75v3.5a1.25 1.25 0 0 1-2.4.5M12 7.5a1.4 1.4 0 0 0-1.3-.5c-.8 0-1.3.4-1.3 1s.5.9 1.3 1.1 1.3.5 1.3 1.1-.5 1.05-1.3 1.05a1.6 1.6 0 0 1-1.45-.65" />"##
        }
        "file-react" => {
            r##"<ellipse cx="8" cy="8" rx="6.25" ry="2.4" /><ellipse cx="8" cy="8" rx="6.25" ry="2.4" transform="rotate(60 8 8)" /><ellipse cx="8" cy="8" rx="6.25" ry="2.4" transform="rotate(120 8 8)" /><circle cx="8" cy="8" r=".9" fill="currentColor" stroke="none" />"##
        }
        "file-json" => {
            r##"<path d="M5.5 2.5c-1.5 0-1.75.9-1.75 2.25S3.5 7.6 2.25 8c1.25.4 1.5 1.9 1.5 3.25S4 13.5 5.5 13.5M10.5 2.5c1.5 0 1.75.9 1.75 2.25S12.5 7.6 13.75 8c-1.25.4-1.5 1.9-1.5 3.25S12 13.5 10.5 13.5" />"##
        }
        "file-md" => {
            r##"<rect x="1.5" y="3.5" width="13" height="9" rx="1.5" /><path d="M4 10.25v-4.5l2 2.25 2-2.25v4.5M11 5.75v4.5M9.5 8.75L11 10.25l1.5-1.5" />"##
        }
        "file-config" => {
            r##"<path d="M2.5 4.5h3M9.5 4.5h4M2.5 8h7M13 8h.5M2.5 11.5h1.5M7.5 11.5h6" /><circle cx="7.5" cy="4.5" r="1.5" /><circle cx="11.25" cy="8" r="1.5" /><circle cx="5.75" cy="11.5" r="1.5" />"##
        }
        "file-css" => r##"<path d="M6.5 2.5L5 13.5M11.5 2.5L10 13.5M3 6h10.5M2.5 10H13" />"##,
        "file-html" => r##"<path d="M5.5 4L2 8l3.5 4M10.5 4L14 8l-3.5 4M9 3l-2 10" />"##,
        "file-py" => {
            r##"<path d="M8 2.25H6.75A2 2 0 0 0 4.75 4.25V6h4.5M4.75 6h-.5A2 2 0 0 0 2.25 8v1.25a2 2 0 0 0 2 2H5.5V9.5a1.5 1.5 0 0 1 1.5-1.5h2.25a1.5 1.5 0 0 0 1.5-1.5V4.25a2 2 0 0 0-2-2H8" /><path d="M8 13.75h1.25a2 2 0 0 0 2-2V10h-4.5M11.25 10h.5a2 2 0 0 0 2-2V6.75a2 2 0 0 0-2-2H10.5" /><circle cx="6.75" cy="4" r=".6" fill="currentColor" stroke="none" /><circle cx="9.25" cy="12" r=".6" fill="currentColor" stroke="none" />"##
        }
        "file-image" => {
            r##"<rect x="2" y="2.75" width="12" height="10.5" rx="1.5" /><circle cx="5.75" cy="6.25" r="1.25" /><path d="M2.5 12.25L6.25 8.5l2.5 2.5 2-2 3 3" />"##
        }
        "file-lock" => {
            r##"<rect x="3.25" y="7" width="9.5" height="7" rx="1.25" /><path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2M8 9.75v1.5" />"##
        }
        "file-shell" => {
            r##"<rect x="1.75" y="2.75" width="12.5" height="10.5" rx="1.5" /><path d="M4.5 6.25L6.75 8 4.5 9.75M8.5 10.25h3" />"##
        }
        "file-git" => {
            r##"<path d="M8 1.75L14.25 8 8 14.25 1.75 8z" /><circle cx="6.5" cy="6.5" r="1" /><circle cx="9.5" cy="9.5" r="1" /><path d="M7.2 7.2l1.6 1.6M6.5 7.5v3" />"##
        }
        "file-text" => {
            r##"<path d="M4 2h5l3 3v9H4z" /><path d="M9 2v3h3M6 8.25h4M6 10.75h4" />"##
        }
        "file-plus" => r##"<path d="M9 2H4v12h5M9 2l3 3v2.5M9 2v3h3" /><path d="M12 10v4M10 12h4" />"##,
        "folder-plus" => {
            r##"<path d="M8.5 13H2V4h4l1.5 1.5H14V8.5" /><path d="M12 10v4M10 12h4" />"##
        }
        "collapse" => r##"<path d="M5 2.5l3 3 3-3M5 13.5l3-3 3 3M2.5 8h11" />"##,
        "arrow-up" => r##"<path d="M8 13V3M4 7l4-4 4 4" />"##,
        "arrow-down" => r##"<path d="M8 3v10M4 9l4 4 4-4" />"##,
        "cloud" => {
            r##"<path d="M4.5 12.5a3 3 0 0 1-.4-6A4 4 0 0 1 11.8 5.6 3.5 3.5 0 0 1 11.5 12.5z" />"##
        }
        "minus" => r##"<path d="M3 8h10" />"##,
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
        "claude" => {
            r##"<g transform="scale(.6666667)" fill="currentColor" stroke="none"><path d="m4.7144 15.9555 4.7174-2.6471.079-.2307-.079-.1275h-.2307l-.7893-.0486-2.6956-.0729-2.3375-.0971-2.2646-.1214-.5707-.1215-.5343-.7042.0546-.3522.4797-.3218.686.0608 1.5179.1032 2.2767.1578 1.6514.0972 2.4468.255h.3886l.0546-.1579-.1336-.0971-.1032-.0972L6.973 9.8356l-2.55-1.6879-1.3356-.9714-.7225-.4918-.3643-.4614-.1578-1.0078.6557-.7225.8803.0607.2246.0607.8925.686 1.9064 1.4754 2.4893 1.8336.3643.3035.1457-.1032.0182-.0728-.164-.2733-1.3539-2.4467-1.445-2.4893-.6435-1.032-.17-.6194c-.0607-.255-.1032-.4674-.1032-.7285L6.287.1335 6.6997 0l.9957.1336.419.3642.6192 1.4147 1.0018 2.2282 1.5543 3.0296.4553.8985.2429.8318.091.255h.1579v-.1457l.1275-1.706.2368-2.0947.2307-2.6957.0789-.7589.3764-.9107.7468-.4918.5828.2793.4797.686-.0668.4433-.2853 1.8517-.5586 2.9021-.3643 1.9429h.2125l.2429-.2429.9835-1.3053 1.6514-2.0643.7286-.8196.85-.9046.5464-.4311h1.0321l.759 1.1293-.34 1.1657-1.0625 1.3478-.8804 1.1414-1.2628 1.7-.7893 1.36.0729.1093.1882-.0183 2.8535-.607 1.5421-.2794 1.8396-.3157.8318.3886.091.3946-.3278.8075-1.967.4857-2.3072.4614-3.4364.8136-.0425.0304.0486.0607 1.5482.1457.6618.0364h1.621l3.0175.2247.7892.522.4736.6376-.079.4857-1.2142.6193-1.6393-.3886-3.825-.9107-1.3113-.3279h-.1822v.1093l1.0929 1.0686 2.0035 1.8092 2.5075 2.3314.1275.5768-.3218.4554-.34-.0486-2.2039-1.6575-.85-.7468-1.9246-1.621h-.1275v.17l.4432.6496 2.3436 3.5214.1214 1.0807-.17.3521-.6071.2125-.6679-.1214-1.3721-1.9246L14.38 17.959l-1.1414-1.9428-.1397.079-.674 7.2552-.3156.3703-.7286.2793-.6071-.4614-.3218-.7468.3218-1.4753.3886-1.9246.3157-1.53.2853-1.9004.17-.6314-.0121-.0425-.1397.0182-1.4328 1.9672-2.1796 2.9446-1.7243 1.8456-.4128.164-.7164-.3704.0667-.6618.4008-.5889 2.386-3.0357 1.4389-1.882.929-1.0868-.0062-.1579h-.0546l-6.3385 4.1164-1.1293.1457-.4857-.4554.0608-.7467.2307-.2429 1.9064-1.3114Z" /></g>"##
        }
        "codex" => {
            r##"<g transform="scale(.6666667)" fill="currentColor" stroke="none"><path d="M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654 2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z" /></g>"##
        }
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
    let image = gtk::Image::new();
    image.set_pixel_size(size);
    let paint = move |image: &gtk::Image| {
        let color = image.color();
        let background = image
            .style_context()
            .lookup_color("console")
            .map(|color| color.to_string())
            .unwrap_or_else(|| "#141416".into());
        let geometry = if geometry.contains("#141416") {
            std::borrow::Cow::Owned(geometry.replace("#141416", &background))
        } else {
            std::borrow::Cow::Borrowed(geometry)
        };
        let document = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" color="{color}" stroke="{color}" stroke-width="{stroke}" stroke-linecap="round" stroke-linejoin="round">{geometry}</svg>"##
        );
        image.set_paintable(Some(&rendered(document)));
    };
    // GTK 4.22 SVG paintables do not inherit currentColor from their widget.
    image.connect_map(paint);
    image.connect_state_flags_changed(move |image, _| paint(image));
    image
}

/// Register the bundled mark with GTK without changing the desktop icon theme.
pub fn install_app_icon(window: &gtk::ApplicationWindow) {
    let directory = glib::user_cache_dir().join("relay-v4/icons");
    let path = directory.join("com.quietsoftware.Relay4.svg");
    let bytes = include_bytes!("../resources/com.quietsoftware.Relay4.svg");
    let result = std::fs::create_dir_all(&directory).and_then(|_| {
        if std::fs::read(&path).ok().as_deref() == Some(bytes.as_slice()) {
            Ok(())
        } else {
            std::fs::write(&path, bytes)
        }
    });
    if let Err(error) = result {
        tracing::warn!(%error, "Could not load Relay icon");
        return;
    }
    gtk::IconTheme::for_display(&gtk::prelude::WidgetExt::display(window))
        .add_search_path(&directory);
    gtk::Window::set_default_icon_name("com.quietsoftware.Relay4");
    window.set_icon_name(Some("com.quietsoftware.Relay4"));
}
