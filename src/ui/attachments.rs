//! Showing `![[embeds]]` beneath the line that names them.
//!
//! # Why overlays and not child anchors
//!
//! `GtkTextChildAnchor` puts a real `U+FFFC` character in the buffer. Every
//! offset in this app — the scanner's spans, the marker ranges, the link
//! ranges — assumes the buffer holds exactly the file's text, so an anchor
//! would shift everything after it and quietly corrupt the note on save.
//!
//! `gtk_text_view_add_overlay` places a widget at a *buffer coordinate*
//! instead, touching no text at all, and GTK moves it as the view scrolls. The
//! cost is that the overlay does not reflow text, so the room for it is made
//! separately: a tag on the embed's line carrying `pixels-below-lines`. That is
//! why the displayed height is fixed rather than the image's own — the space
//! is reserved by a tag before the picture is measured.

use std::path::{Path, PathBuf};

use gtk::glib;
use gtk::prelude::*;

use crate::model::markdown::{Parsed, Style};

/// How tall an embedded image is drawn. Fixed, because the space for it is
/// reserved by a text tag that cannot know the picture's aspect ratio.
const IMAGE_HEIGHT: i32 = 240;
/// The gap under the picture, so the next paragraph does not touch it.
const GAP: i32 = 8;
/// A non-image attachment is a chip, which is one line tall.
const CHIP_HEIGHT: i32 = 36;

/// Tags reserving the room an overlay is drawn into.
pub const IMAGE_SPACE: &str = "md-embed-image-space";
pub const FILE_SPACE: &str = "md-embed-file-space";

/// Extensions rendered as pictures. Anything else gets a chip — guessing at a
/// file's contents from its name is fine for deciding how to *show* it, and
/// wrong for anything that matters.
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "avif"];

pub fn install_tags(buffer: &gtk::TextBuffer) {
    let table = buffer.tag_table();
    for (name, height) in [
        (IMAGE_SPACE, IMAGE_HEIGHT + GAP),
        (FILE_SPACE, CHIP_HEIGHT + GAP),
    ] {
        if table.lookup(name).is_some() {
            continue;
        }
        let tag = gtk::TextTag::builder().name(name).build();
        tag.set_pixels_below_lines(height);
        table.add(&tag);
    }
}

fn is_image(name: &str) -> bool {
    name.rsplit('.')
        .next()
        .map(str::to_lowercase)
        .is_some_and(|extension| IMAGE_EXTENSIONS.contains(&extension.as_str()))
}

/// Where an embed target lives, if anywhere.
///
/// `attachments/` first, since that is where Brain puts what you drop, then the
/// vault root, so a note written elsewhere that references a file beside it
/// still shows it.
pub fn resolve(root: &Path, target: &str) -> Option<PathBuf> {
    // A target is a filename, not a path to follow out of the vault.
    if target.contains("..") {
        return None;
    }
    let candidates = [
        root.join(crate::model::vault::ATTACHMENTS_DIR).join(target),
        root.join(target),
    ];
    candidates.into_iter().find(|path| path.is_file())
}

/// Rebuild the overlays for every embed in the note.
///
/// Returns the widgets, which the caller keeps so it can remove them next time
/// — a `GtkTextView` has no way to enumerate its own overlays.
pub fn refresh(
    view: &gtk::TextView,
    parsed: &Parsed,
    root: Option<&Path>,
    previous: Vec<gtk::Widget>,
) -> Vec<gtk::Widget> {
    for widget in previous {
        view.remove(&widget);
    }

    let buffer = view.buffer();
    let table = buffer.tag_table();
    let length = buffer.char_count();

    // Clear the reserved space before deciding where to put it back.
    if let (Some(image), Some(file)) = (table.lookup(IMAGE_SPACE), table.lookup(FILE_SPACE)) {
        let start = buffer.start_iter();
        let end = buffer.end_iter();
        buffer.remove_tag(&image, &start, &end);
        buffer.remove_tag(&file, &start, &end);
    }

    let Some(root) = root else {
        return Vec::new();
    };

    let mut widgets = Vec::new();
    for span in &parsed.spans {
        if span.style != Style::Embed {
            continue;
        }
        let start = (span.start as i32).clamp(0, length);
        let end = (span.end as i32).clamp(0, length);
        let target: String = buffer
            .text(
                &buffer.iter_at_offset(start),
                &buffer.iter_at_offset(end),
                true,
            )
            .to_string();
        // "diagram.png|300" — the size hint is not part of the filename.
        let target = target.split('|').next().unwrap_or("").trim().to_string();
        if target.is_empty() {
            continue;
        }

        let path = resolve(root, &target);
        let image = is_image(&target);
        let drawn = match &path {
            Some(path) if image => picture(path),
            _ => None,
        };
        let is_picture = drawn.is_some();
        let widget = match drawn {
            Some(widget) => widget,
            None => chip(&target, path.as_deref()),
        };

        // Reserve the room, on the line the embed sits on.
        let space = if is_picture { IMAGE_SPACE } else { FILE_SPACE };
        if let Some(tag) = table.lookup(space) {
            // From the *line's* first character, not the span's. GTK reads
            // paragraph attributes like `pixels-below-lines` from the start of
            // the line, so a tag beginning three characters in — after the
            // `![[` — reserves nothing at all.
            let mut line_start = buffer.iter_at_offset(start);
            line_start.set_line_offset(0);
            let mut line_end = buffer.iter_at_offset(end);
            if !line_end.ends_line() {
                line_end.forward_to_line_end();
            }
            buffer.apply_tag(&tag, &line_start, &line_end);
        }

        // Buffer coordinates, so GTK keeps the overlay in place as the view
        // scrolls.
        let y = overlay_y(view, &buffer, end, is_picture);
        view.add_overlay(&widget, view.left_margin(), y);
        widgets.push(widget.upcast());
    }
    widgets
}

/// Move the overlays back where they belong after the text reflows.
pub fn reposition(view: &gtk::TextView, parsed: &Parsed, widgets: &[gtk::Widget]) {
    let buffer = view.buffer();
    let length = buffer.char_count();
    let embeds = parsed
        .spans
        .iter()
        .filter(|span| span.style == Style::Embed);

    for (span, widget) in embeds.zip(widgets) {
        let end = (span.end as i32).clamp(0, length);
        // A picture is taller than a chip, so which one this is decides how
        // much of the line's box is reserved space rather than text.
        let is_picture = widget.is::<gtk::Picture>();
        let y = overlay_y(view, &buffer, end, is_picture);
        view.move_overlay(widget, view.left_margin(), y);
    }
}

/// The top of the space reserved beneath an embed's line.
///
/// Taken from the *line's* box rather than the character's: `iter_location`
/// reports the glyph, which is shorter than the line and leaves the picture
/// overlapping the text it belongs to. `line_yrange` includes the room the
/// spacing tag reserved, so subtracting that room lands exactly on top of it.
fn overlay_y(view: &gtk::TextView, buffer: &gtk::TextBuffer, offset: i32, picture: bool) -> i32 {
    let iter = buffer.iter_at_offset(offset);
    let (top, height) = view.line_yrange(&iter);
    let reserved = if picture {
        IMAGE_HEIGHT + GAP
    } else {
        CHIP_HEIGHT + GAP
    };
    top + height - reserved
}

/// The widest an embedded image is drawn, so a panorama does not run off the
/// side of a view that cannot scroll horizontally.
const MAX_WIDTH: i32 = 560;

/// A picture at an explicit size, or `None` if the file is not an image after
/// all — a `.png` that is not a PNG is a broken file, not a broken app.
///
/// The size has to be set on both axes. An overlay is allocated its *minimum*
/// size, and a `GtkPicture` that can shrink has a minimum width of zero, which
/// draws nothing at all.
fn picture(path: &Path) -> Option<gtk::Widget> {
    let texture = gtk::gdk::Texture::from_filename(path).ok()?;
    let (width, height) = (texture.width().max(1), texture.height().max(1));
    let scaled = (width * IMAGE_HEIGHT / height).clamp(1, MAX_WIDTH);

    let picture = gtk::Picture::for_paintable(&texture);
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_halign(gtk::Align::Start);
    picture.set_size_request(scaled, IMAGE_HEIGHT);
    picture.set_tooltip_text(path.file_name().and_then(|name| name.to_str()));
    Some(picture.upcast())
}

/// A non-image attachment, or one whose file is missing.
fn chip(target: &str, path: Option<&Path>) -> gtk::Widget {
    let icon = if path.is_some() {
        "mail-attachment-symbolic"
    } else {
        // A missing file is said so plainly rather than shown as a broken
        // image, which looks like the app failing rather than the file being
        // absent.
        "image-missing-symbolic"
    };

    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&gtk::Image::from_icon_name(icon));
    content.append(&gtk::Label::new(Some(target)));

    let button = gtk::Button::builder()
        .child(&content)
        .halign(gtk::Align::Start)
        .build();
    button.add_css_class("pill");

    match path {
        Some(path) => {
            let uri = glib::filename_to_uri(path, None)
                .map(|uri| uri.to_string())
                .unwrap_or_default();
            button.set_tooltip_text(Some(&format!("Open {target}")));
            button.connect_clicked(move |button| {
                let launcher = gtk::UriLauncher::new(&uri);
                let window = button.root().and_downcast::<gtk::Window>();
                launcher.launch(window.as_ref(), gtk::gio::Cancellable::NONE, |_| {});
            });
        }
        None => {
            button.set_tooltip_text(Some(&format!("{target} is not in the vault")));
            button.set_sensitive(false);
        }
    }
    button.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_are_recognised_by_extension_case_insensitively() {
        for name in ["a.png", "a.PNG", "photo.jpeg", "b.svg", "c.webp"] {
            assert!(is_image(name), "{name}");
        }
        for name in ["notes.pdf", "archive.zip", "noextension", "a.png.txt"] {
            assert!(!is_image(name), "{name}");
        }
    }

    #[test]
    fn resolution_prefers_the_attachments_folder() {
        let root = tempfile::tempdir().expect("temp dir");
        let attachments = root.path().join("attachments");
        std::fs::create_dir_all(&attachments).expect("dir");
        std::fs::write(attachments.join("d.png"), b"a").expect("write");
        std::fs::write(root.path().join("d.png"), b"b").expect("write");

        assert_eq!(
            resolve(root.path(), "d.png"),
            Some(attachments.join("d.png"))
        );
    }

    #[test]
    fn a_file_beside_the_note_still_resolves() {
        let root = tempfile::tempdir().expect("temp dir");
        std::fs::write(root.path().join("beside.png"), b"a").expect("write");
        assert_eq!(
            resolve(root.path(), "beside.png"),
            Some(root.path().join("beside.png"))
        );
    }

    #[test]
    fn a_missing_file_resolves_to_nothing() {
        let root = tempfile::tempdir().expect("temp dir");
        assert_eq!(resolve(root.path(), "gone.png"), None);
    }

    #[test]
    fn an_embed_cannot_climb_out_of_the_vault() {
        // "![[../../.ssh/id_rsa]]" is a note asking to display a file it has
        // no business displaying.
        let root = tempfile::tempdir().expect("temp dir");
        let outside = root.path().join("outside.png");
        std::fs::write(&outside, b"a").expect("write");
        let inside = root.path().join("vault");
        std::fs::create_dir_all(&inside).expect("dir");

        assert_eq!(resolve(&inside, "../outside.png"), None);
    }
}
