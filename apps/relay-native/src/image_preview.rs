use super::*;
use base64::Engine as _;

const MAX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_EDGE: i32 = 4096;

pub(super) fn is_image(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "ico" | "svg"
            )
        })
}

pub(super) struct Preview {
    pub root: gtk::Box,
    pub picture: gtk::Picture,
    title: gtk::Label,
    message: gtk::Label,
}
impl Preview {
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let title = label("", "mono");
        title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        title.set_hexpand(true);
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        heading.add_css_class("toolbar");
        heading.append(&title);
        root.append(&heading);
        let picture = gtk::Picture::new();
        picture.set_widget_name("project-image");
        picture.set_can_shrink(true);
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        picture.set_margin_top(16);
        picture.set_margin_bottom(16);
        picture.set_margin_start(16);
        picture.set_margin_end(16);
        root.append(&picture);
        let message = label("", "dim");
        message.set_widget_name("project-image-message");
        message.set_wrap(true);
        message.set_selectable(true);
        message.set_justify(gtk::Justification::Center);
        message.set_xalign(0.5);
        message.set_halign(gtk::Align::Center);
        message.set_valign(gtk::Align::Center);
        message.set_vexpand(true);
        message.set_margin_start(24);
        message.set_margin_end(24);
        root.append(&message);
        Self {
            root,
            picture,
            title,
            message,
        }
    }

    fn loading(&self, path: &str) {
        self.title.set_text(path);
        self.title.set_tooltip_text(Some(path));
        self.picture.set_alternative_text(Some(path));
        self.clear();
        self.message.set_text("Loading image…");
        self.message.set_visible(true);
        self.picture.set_visible(false);
    }

    pub fn clear(&self) {
        self.picture.set_paintable(None::<&gtk::gdk::Paintable>);
    }
}

struct Decoded {
    pixels: Vec<u8>,
    width: i32,
    height: i32,
    stride: usize,
    alpha: bool,
    original: (i32, i32),
}

// Decode on a worker and transfer only pixel bytes to GTK. Limit the displayed
// resolution so a large asset cannot allocate an equally large GPU texture.
fn decode(value: Value) -> Result<Decoded, String> {
    decode_to(value, MAX_EDGE)
}

fn decode_to(value: Value, max_edge: i32) -> Result<Decoded, String> {
    if value["truncated"] == true {
        return Err("Image exceeds the 16 MiB preview limit.".into());
    }
    let bytes = if let Some(encoded) = value["bytes_b64"].as_str() {
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| e.to_string())?
    } else if let Some(text) = value["text"].as_str() {
        text.as_bytes().to_vec()
    } else {
        return Err("The file did not contain image data.".into());
    };
    let loader = gtk::gdk_pixbuf::PixbufLoader::new();
    let dimensions = Rc::new(Cell::new((0, 0)));
    let prepared = dimensions.clone();
    loader.connect_size_prepared(move |loader, width, height| {
        prepared.set((width, height));
        let scale = (max_edge as f64 / width.max(height).max(1) as f64).min(1.);
        loader.set_size(
            (width as f64 * scale).max(1.) as i32,
            (height as f64 * scale).max(1.) as i32,
        );
    });
    let read: Result<(), String> = bytes.chunks(8192).try_for_each(|chunk| {
        loader.write(chunk).map_err(|e| e.to_string())?;
        let (width, height) = dimensions.get();
        if i64::from(width) * i64::from(height) > 100_000_000 {
            return Err("Image dimensions exceed the 100 megapixel preview limit.".into());
        }
        Ok(())
    });
    let closed = loader.close().map_err(|e| e.to_string());
    read?;
    closed?;
    let pixbuf = loader
        .pixbuf()
        .ok_or("The image format could not be decoded.")?;
    let pixbuf = pixbuf.apply_embedded_orientation().unwrap_or(pixbuf);
    Ok(Decoded {
        pixels: pixbuf.read_pixel_bytes().as_ref().to_vec(),
        width: pixbuf.width(),
        height: pixbuf.height(),
        stride: pixbuf.rowstride() as usize,
        alpha: pixbuf.has_alpha(),
        original: dimensions.get(),
    })
}

impl Decoded {
    fn texture(self) -> gtk::gdk::MemoryTexture {
        let format = if self.alpha {
            gtk::gdk::MemoryFormat::R8g8b8a8
        } else {
            gtk::gdk::MemoryFormat::R8g8b8
        };
        gtk::gdk::MemoryTexture::new(
            self.width,
            self.height,
            format,
            &glib::Bytes::from_owned(self.pixels),
            self.stride,
        )
    }
}

/// The longest edge of a hover thumbnail.
const THUMB_EDGE: i32 = 256;
/// Hover thumbnails kept per window; a tree full of assets is browsed, not memorised.
const THUMB_CACHE: usize = 64;
const HOVER_DELAY: std::time::Duration = std::time::Duration::from_millis(350);

#[derive(Clone)]
struct Thumb {
    texture: Option<gtk::gdk::MemoryTexture>,
    caption: String,
}

type ThumbCache = (Vec<String>, std::collections::HashMap<String, Thumb>);
thread_local! {
    static THUMBS: RefCell<ThumbCache> = RefCell::new((Vec::new(), std::collections::HashMap::new()));
}

fn cached(key: &str) -> Option<Thumb> {
    THUMBS.with(|cache| cache.borrow().1.get(key).cloned())
}

fn remember(key: String, thumb: Thumb) {
    THUMBS.with(|cache| {
        let (order, map) = &mut *cache.borrow_mut();
        if map.insert(key.clone(), thumb).is_none() {
            order.push(key);
        }
        while order.len() > THUMB_CACHE {
            let oldest = order.remove(0);
            map.remove(&oldest);
        }
    });
}

fn caption(decoded: &Decoded, size: u64, path: &str) -> String {
    let format = std::path::Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_uppercase();
    let size = if size >= 1048576 {
        format!("{:.1} MiB", size as f64 / 1048576.)
    } else {
        format!("{:.0} KiB", (size as f64 / 1024.).max(1.))
    };
    format!("{} × {} · {size} · {format}", decoded.original.0, decoded.original.1)
}

/// Fill a hover card from a thumbnail, or its error.
fn show_thumb(card: &gtk::Box, picture: &gtk::Picture, note: &gtk::Label, thumb: &Thumb) {
    card.remove_css_class("loading");
    match &thumb.texture {
        Some(texture) => {
            picture.set_paintable(Some(texture));
            // A tiny icon is drawn larger so its shape reads, never past 4×.
            let (width, height) = (texture.width(), texture.height());
            let scale = (96. / width.max(height).max(1) as f64).clamp(1., 4.);
            picture.set_size_request((width as f64 * scale) as i32, (height as f64 * scale) as i32);
            picture.set_visible(true);
            note.remove_css_class("error");
        }
        None => {
            picture.set_visible(false);
            note.add_css_class("error");
        }
    }
    note.set_text(&thumb.caption);
}

impl Editor {
    /// Show a delayed thumbnail beside any row naming an image, as VS Code's explorer does.
    /// `stamp` (size and modification time, when the caller has them) keeps the cache honest.
    pub(super) fn bind_image_hover(
        self: &Rc<Self>,
        ui: &Rc<Ui>,
        row: &impl IsA<gtk::Widget>,
        path: &str,
        stamp: String,
    ) {
        if !is_image(path) {
            return;
        }
        let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
        let shown: Rc<RefCell<Option<gtk::Popover>>> = Rc::default();
        let hide = {
            let pending = pending.clone();
            let shown = shown.clone();
            move || {
                if let Some(source) = pending.borrow_mut().take() {
                    source.remove();
                }
                let popover = shown.borrow_mut().take();
                if let Some(popover) = popover {
                    popover.popdown();
                }
            }
        };
        let motion = gtk::EventControllerMotion::new();
        let target = row.as_ref().downgrade();
        let editor = Rc::downgrade(self);
        let weak_ui = Rc::downgrade(ui);
        let path = path.to_owned();
        motion.connect_enter(move |_, _, _| {
            if pending.borrow().is_some() || shown.borrow().is_some() {
                return;
            }
            let fired = pending.clone();
            let shown = shown.clone();
            let target = target.clone();
            let editor = editor.clone();
            let weak_ui = weak_ui.clone();
            let path = path.clone();
            let stamp = stamp.clone();
            let source = glib::timeout_add_local_once(HOVER_DELAY, move || {
                // The source has fired; it must not be removed again.
                fired.borrow_mut().take();
                let (Some(row), Some(editor), Some(ui)) =
                    (target.upgrade(), editor.upgrade(), weak_ui.upgrade())
                else {
                    return;
                };
                if !row.is_mapped() {
                    return;
                }
                let popover = gtk::Popover::new();
                popover.add_css_class("image-hover");
                popover.set_autohide(false);
                popover.set_has_arrow(false);
                popover.set_can_focus(false);
                popover.set_position(gtk::PositionType::Right);
                popover.set_offset(6, 0);
                let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
                card.add_css_class("image-hover-card");
                card.add_css_class("loading");
                let name = label(
                    std::path::Path::new(&path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(&path),
                    "image-hover-name",
                );
                name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                name.set_max_width_chars(36);
                card.append(&name);
                let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
                frame.add_css_class("image-hover-frame");
                frame.set_halign(gtk::Align::Center);
                let picture = gtk::Picture::new();
                picture.set_can_shrink(false);
                picture.set_content_fit(gtk::ContentFit::Contain);
                picture.set_visible(false);
                picture.set_alternative_text(Some(&path));
                frame.append(&picture);
                card.append(&frame);
                let note = label("Loading preview…", "image-hover-meta");
                note.set_wrap(true);
                note.set_max_width_chars(36);
                card.append(&note);
                popover.set_child(Some(&card));
                popover.set_parent(&row);
                let closing = shown.clone();
                popover.connect_closed(move |popover| {
                    closing.borrow_mut().take();
                    popover.unparent();
                });
                *shown.borrow_mut() = Some(popover.clone());
                popover.popup();
                let project = ui.project.get();
                let worktree = editor.worktree.borrow().clone();
                let key = format!("{project}\0{worktree}\0{path}\0{stamp}");
                if let Some(thumb) = cached(&key) {
                    show_thumb(&card, &picture, &note, &thumb);
                    return;
                }
                let path = path.clone();
                glib::spawn_future_local(async move {
                    let result = crate::client::Client::image_read(&ui.rt, ui.path.clone(),
                        json!({"project_id":project,"worktree":optional_scope(&worktree),"path":path,"max_bytes":MAX_BYTES}))
                        .await
                        .map_err(|err| err.to_string());
                    let size = result.as_ref().ok().and_then(|v| v["size"].as_u64()).unwrap_or(0);
                    let decoded = match result {
                        Ok(value) => ui
                            .rt
                            .spawn_blocking(move || decode_to(value, THUMB_EDGE))
                            .await
                            .map_err(|e| e.to_string())
                            .and_then(|v| v),
                        Err(error) => Err(error),
                    };
                    let thumb = match decoded {
                        Ok(decoded) => Thumb {
                            caption: caption(&decoded, size, &path),
                            texture: Some(decoded.texture()),
                        },
                        Err(error) => Thumb { texture: None, caption: format!("No preview · {error}") },
                    };
                    remember(key, thumb.clone());
                    show_thumb(&card, &picture, &note, &thumb);
                });
            });
            *pending.borrow_mut() = Some(source);
        });
        let leave = hide.clone();
        motion.connect_leave(move |_| leave());
        row.add_controller(motion);
        // A tree rebuild, or the click that opens the file, must not leave a card floating.
        let unmapped = hide.clone();
        row.connect_unmap(move |_| unmapped());
        let click = gtk::GestureClick::new();
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        click.connect_pressed(move |_, _, _, _| hide());
        row.add_controller(click);
    }

    pub(super) fn open_image(self: &Rc<Self>, ui: &Rc<Ui>, path: String) {
        self.revision.set(self.revision.get() + 1);
        let revision = self.revision.get();
        let project = ui.project.get();
        let worktree = self.worktree.borrow().clone();
        self.image_mode.set(true);
        self.diff.set(false);
        self.before_scroll.set_visible(false);
        self.buffer.set_text("");
        self.buffer.set_modified(false);
        self.original.borrow_mut().clear();
        *self.path.borrow_mut() = path.clone();
        self.project.set(project);
        self.image.loading(&path);
        self.position.set_text("Image preview");
        self.find_bar.set_visible(false);
        self.set_busy(true);
        let e = self.clone();
        let ui = ui.clone();
        glib::spawn_future_local(async move {
            let result = crate::client::Client::image_read(&ui.rt, ui.path.clone(),
                json!({"project_id":project,"worktree":optional_scope(&worktree),"path":path,"max_bytes":MAX_BYTES}))
                .await.map_err(|err| err.to_string());
            if e.revision.get() != revision || !e.matches(&ui, project, &worktree) {
                return;
            }
            let size = result
                .as_ref()
                .ok()
                .and_then(|v| v["size"].as_u64())
                .unwrap_or(0);
            let decoded = match result {
                Ok(value) => ui
                    .rt
                    .spawn_blocking(move || decode(value))
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|v| v),
                Err(error) => Err(error),
            };
            if e.revision.get() != revision || !e.matches(&ui, project, &worktree) {
                return;
            }
            e.set_busy(false);
            match decoded {
                Ok(decoded) => {
                    let original = decoded.original;
                    let texture = decoded.texture();
                    e.image.picture.set_paintable(Some(&texture));
                    e.image.picture.set_visible(true);
                    e.image.message.set_visible(false);
                    e.position.set_text(&format!(
                        "{} × {} px · {:.1} MiB · Image preview",
                        original.0,
                        original.1,
                        size as f64 / 1048576.
                    ));
                }
                Err(error) => {
                    e.image
                        .message
                        .set_text(&format!("Cannot preview image\n{error}"));
                    e.position.set_text("Image preview unavailable");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raster_formats_decode_without_a_display_and_large_images_are_scaled() {
        let pixbuf =
            gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 5000, 8)
                .unwrap();
        pixbuf.fill(0x3377aaff);
        for format in ["png", "jpeg", "bmp", "tiff"] {
            let bytes = pixbuf.save_to_bufferv(format, &[]).unwrap();
            let image = decode(
                json!({"bytes_b64":base64::engine::general_purpose::STANDARD.encode(bytes)}),
            )
            .unwrap();
            assert_eq!(image.original, (5000, 8));
            assert_eq!(image.width, MAX_EDGE);
            assert!(image.height > 0 && image.height <= 8);
        }
    }

    #[test]
    fn hover_thumbnails_are_bounded_and_captioned_with_the_original_size() {
        let pixbuf =
            gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 1200, 600)
                .unwrap();
        pixbuf.fill(0x3377aaff);
        let bytes = pixbuf.save_to_bufferv("png", &[]).unwrap();
        let thumb = decode_to(
            json!({"bytes_b64":base64::engine::general_purpose::STANDARD.encode(&bytes)}),
            THUMB_EDGE,
        )
        .unwrap();
        assert_eq!((thumb.width, thumb.height), (THUMB_EDGE, THUMB_EDGE / 2));
        assert_eq!(caption(&thumb, 3 * 1048576 / 2, "art/hero.png"), "1200 × 600 · 1.5 MiB · PNG");
        assert_eq!(caption(&thumb, 10, "a.webp"), "1200 × 600 · 1 KiB · WEBP");
        for n in 0..THUMB_CACHE + 5 {
            remember(n.to_string(), Thumb { texture: None, caption: String::new() });
        }
        assert!(cached("0").is_none() && cached(&(THUMB_CACHE + 4).to_string()).is_some());
        THUMBS.with(|cache| assert_eq!(cache.borrow().1.len(), THUMB_CACHE));
    }

    #[test]
    fn svg_and_invalid_or_oversized_images_have_explicit_results() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="30" height="20"><rect width="30" height="20" fill="#3377aa"/></svg>"##;
        assert_eq!(decode(json!({"text":svg})).unwrap().original, (30, 20));
        assert!(decode(json!({"text":"not an image"})).is_err());
        assert!(decode(json!({"truncated":true}))
            .err()
            .unwrap()
            .contains("16 MiB"));
        assert!(is_image("ART/Preview.PNG") && is_image("photo.JPEG") && is_image("asset.webp"));
        assert!(!is_image("image.png.rs"));
    }
}
