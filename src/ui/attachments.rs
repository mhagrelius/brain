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
//! separately: a tag on the embed's line carrying `pixels-below-lines`. Each
//! embed is measured first and then given a tag reserving exactly its height,
//! so a picture sits in a box its own shape rather than a fixed one it has to
//! be letterboxed into.

use std::path::{Path, PathBuf};

use gtk::glib;
use gtk::prelude::*;

use crate::model::markdown::{Parsed, Style};

/// The box an embedded image is drawn to fit inside, keeping its aspect ratio.
/// The width stops a panorama running off a view that cannot scroll sideways;
/// the height stops a portrait photograph filling the window.
const MAX_WIDTH: i32 = 560;
const MAX_HEIGHT: i32 = 360;
/// The gap under the picture, so the next paragraph does not touch it.
const GAP: i32 = 8;
/// A non-image attachment is a chip, which is one line tall.
const CHIP_HEIGHT: i32 = 36;

/// Tags reserving the room an overlay is drawn into, one per height in use —
/// `pixels-below-lines` is a number on a tag, and each picture reserves its
/// own. Made on demand and named for the height they reserve, so a note with
/// three images of the same shape shares one tag.
const SPACE_PREFIX: &str = "md-embed-space-";

/// The class on an overlay slot, so a test can find what is really on screen.
pub const SLOT: &str = "embed-slot";

/// Extensions rendered as pictures. Anything else gets a chip — guessing at a
/// file's contents from its name is fine for deciding how to *show* it, and
/// wrong for anything that matters.
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "avif"];

/// The tag reserving `height` pixels of room, making it if this is the first
/// embed that tall.
fn space_tag(buffer: &gtk::TextBuffer, height: i32) -> gtk::TextTag {
    let table = buffer.tag_table();
    let name = format!("{SPACE_PREFIX}{height}");
    if let Some(tag) = table.lookup(&name) {
        return tag;
    }
    let tag = gtk::TextTag::builder().name(&name).build();
    tag.set_pixels_below_lines(height + GAP);
    table.add(&tag);
    tag
}

/// Take the reserved room back off every line, before deciding where to put it.
fn clear_space(buffer: &gtk::TextBuffer) {
    let mut tags = Vec::new();
    buffer.tag_table().foreach(|tag| {
        if tag
            .name()
            .is_some_and(|name| name.starts_with(SPACE_PREFIX))
        {
            tags.push(tag.clone());
        }
    });
    let (start, end) = buffer.bounds();
    for tag in tags {
        buffer.remove_tag(&tag, &start, &end);
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

/// Redraw the embeds, filling `slots` and returning it for next time.
///
/// **An overlay cannot be removed.** `gtk_text_view_remove` handles children
/// added at an anchor and the gutter windows; an overlay is parented to a
/// private `GtkTextViewChild` that matches neither, so removing one warns and
/// does nothing, and `unparent` leaves it in the internal list to be allocated
/// and drawn for ever. So the slots are kept and reused: each is an empty box
/// added to the view once, refilled with this scan's picture or chip, and
/// hidden when the note has fewer embeds than the last one did.
pub fn refresh(
    view: &gtk::TextView,
    parsed: &Parsed,
    root: Option<&Path>,
    mut slots: Vec<gtk::Widget>,
) -> Vec<gtk::Widget> {
    let buffer = view.buffer();
    let length = buffer.char_count();

    clear_space(&buffer);

    let Some(root) = root else {
        for slot in &slots {
            slot.set_visible(false);
        }
        return slots;
    };

    let mut used = 0usize;
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
        let content = match drawn {
            Some(widget) => widget,
            None => chip(&target, path.as_deref()),
        };
        let height = match content.height_request() {
            height if height > 0 => height,
            _ => CHIP_HEIGHT,
        };

        // Reserve the room, on the line the embed sits on. From the *line's*
        // first character, not the span's: GTK reads paragraph attributes like
        // `pixels-below-lines` from the start of the line, so a tag beginning
        // three characters in — after the `![[` — reserves nothing at all.
        let mut line_start = buffer.iter_at_offset(start);
        line_start.set_line_offset(0);
        let mut line_end = buffer.iter_at_offset(end);
        if !line_end.ends_line() {
            line_end.forward_to_line_end();
        }
        buffer.apply_tag(&space_tag(&buffer, height), &line_start, &line_end);

        let slot = match slots.get(used) {
            Some(slot) => slot.clone(),
            None => {
                let slot: gtk::Widget = gtk::Box::new(gtk::Orientation::Horizontal, 0).upcast();
                slot.add_css_class(SLOT);
                slot.set_halign(gtk::Align::Start);
                slot.set_valign(gtk::Align::Start);
                view.add_overlay(&slot, view.left_margin(), 0);
                slots.push(slot.clone());
                slot
            }
        };
        fill(&slot, &content, height);
        used += 1;

        // Buffer coordinates, so GTK keeps the overlay in place as the view
        // scrolls.
        let y = overlay_y(view, &buffer, end, height);
        view.move_overlay(&slot, view.left_margin(), y);
    }

    for spare in &slots[used..] {
        spare.set_visible(false);
    }
    slots
}

/// Put this scan's picture or chip in a slot, in place of whatever the last
/// one left there.
fn fill(slot: &gtk::Widget, content: &gtk::Widget, height: i32) {
    let Some(slot) = slot.downcast_ref::<gtk::Box>() else {
        return;
    };
    while let Some(child) = slot.first_child() {
        slot.remove(&child);
    }
    slot.append(content);
    // Read back by `reposition`, which knows where an embed goes but not how
    // tall the thing in it turned out.
    slot.set_size_request(-1, height);
    slot.set_visible(true);
}

/// Move the overlays back where they belong after the text reflows.
pub fn reposition(view: &gtk::TextView, parsed: &Parsed, slots: &[gtk::Widget]) {
    let buffer = view.buffer();
    let length = buffer.char_count();
    let embeds = parsed
        .spans
        .iter()
        .filter(|span| span.style == Style::Embed);
    // Only the filled slots; the spare ones are hidden and belong nowhere.
    let filled = slots.iter().filter(|slot| slot.get_visible());

    for (span, slot) in embeds.zip(filled) {
        let end = (span.end as i32).clamp(0, length);
        let y = overlay_y(view, &buffer, end, slot.height_request().max(CHIP_HEIGHT));
        view.move_overlay(slot, view.left_margin(), y);
    }
}

/// The top of the space reserved beneath an embed's line.
///
/// Taken from the *line's* box rather than the character's: `iter_location`
/// reports the glyph, which is shorter than the line and leaves the picture
/// overlapping the text it belongs to. `line_yrange` includes the room the
/// spacing tag reserved, so subtracting that room lands exactly on top of it.
fn overlay_y(view: &gtk::TextView, buffer: &gtk::TextBuffer, offset: i32, reserved: i32) -> i32 {
    let iter = buffer.iter_at_offset(offset);
    let (top, height) = view.line_yrange(&iter);
    top + height - (reserved + GAP)
}

/// A picture at its own shape, or `None` if the file is not an image after all
/// — a `.png` that is not a PNG is a broken file, not a broken app.
///
/// Fitted inside the box rather than stretched to it, and never scaled up: a
/// small image drawn at its own size is a small image, while one blown up to a
/// fixed height is a blurry one. The size has to be set on both axes, because
/// an overlay is allocated its *minimum* size and a `GtkPicture` that can
/// shrink has a minimum of zero, which draws nothing at all.
fn picture(path: &Path) -> Option<gtk::Widget> {
    let texture = gtk::gdk::Texture::from_filename(path).ok()?;
    let (width, height) = (texture.width().max(1), texture.height().max(1));
    let scale = f64::min(
        f64::from(MAX_WIDTH) / f64::from(width),
        f64::from(MAX_HEIGHT) / f64::from(height),
    )
    .min(1.0);
    let drawn = |side: i32| ((f64::from(side) * scale).round() as i32).max(1);

    let picture = gtk::Picture::for_paintable(&texture);
    picture.set_content_fit(gtk::ContentFit::Fill);
    picture.set_tooltip_text(path.file_name().and_then(|name| name.to_str()));

    // The rounded corners are the frame's: GTK clips a widget's children to
    // its own rounded background, and a `GtkPicture` has no background to
    // round.
    let frame = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    frame.append(&picture);
    frame.add_css_class("embed-image");
    frame.set_overflow(gtk::Overflow::Hidden);
    frame.set_halign(gtk::Align::Start);
    frame.set_size_request(drawn(width), drawn(height));
    Some(frame.upcast())
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
        .height_request(CHIP_HEIGHT)
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
