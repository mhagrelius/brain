//! Turning the scanner's spans into `GtkTextTag`s on a buffer.
//!
//! # The two kinds of tag
//!
//! A **style** tag says how a run of text looks: bigger for a heading, mono for
//! code, accent-coloured for a link. One per [`Style`], installed once.
//!
//! The **marker** tag is the whole editing model. It carries `invisible`, and
//! it is applied to every character that is syntax rather than content. So the
//! note reads as prose while the syntax stays in the file, and the syntax of
//! whatever construct the caret is in comes back for as long as it is there —
//! see [`reveal_markers`].
//!
//! # Colours
//!
//! `GtkTextTag` takes colours, not CSS classes, so these cannot be left to the
//! stylesheet the way the rest of the app is. They are therefore derived from
//! libadwaita's *effective* scheme and accent, and re-derived when either
//! changes. Asking the system for `prefers-color-scheme` instead would be
//! asking the wrong thing — the user can force a scheme per app.

use gtk::gdk::RGBA;
use gtk::prelude::*;

use crate::model::markdown::{Marker, Parsed, Span, Style};

/// The tag applied to syntax characters. Its `invisible` property is what makes
/// the note look rendered.
pub const MARKER: &str = "md-marker";

/// Every style tag's name, so a re-highlight can clear the lot without knowing
/// what was applied where.
/// Names of the per-depth list tags, built once so `clear` can remove them
/// without knowing which depth was applied where.
fn list_tags() -> Vec<String> {
    (0..=crate::model::markdown::MAX_LIST_DEPTH)
        .map(|level| format!("md-list{level}"))
        .collect()
}

const STYLE_TAGS: &[&str] = &[
    "md-h1",
    "md-h2",
    "md-h3",
    "md-h4",
    "md-bold",
    "md-italic",
    "md-strike",
    "md-code",
    "md-codeblock",
    "md-quote",
    "md-link",
    "md-wikilink",
    "md-embed",
    "md-tag",
    "md-task",
    "md-task-done",
    "md-rule",
    "md-frontmatter",
    "md-table",
    "md-table-delimiter",
];

/// The tag for a style. Owned, because the list tags are per-depth.
fn tag_name(style: Style) -> String {
    if let Style::ListItem(level) = style {
        return format!(
            "md-list{}",
            level.min(crate::model::markdown::MAX_LIST_DEPTH)
        );
    }
    static_tag_name(style).to_string()
}

fn static_tag_name(style: Style) -> &'static str {
    match style {
        Style::Heading(1) => "md-h1",
        Style::Heading(2) => "md-h2",
        Style::Heading(3) => "md-h3",
        // 4, 5 and 6 share a treatment: below the third level the difference
        // between sizes is smaller than the noise, and a distinct-but-identical
        // tag would only be honest about the scanner, not about the page.
        Style::Heading(_) => "md-h4",
        Style::Bold => "md-bold",
        Style::Italic => "md-italic",
        Style::Strikethrough => "md-strike",
        Style::Code => "md-code",
        Style::CodeBlock => "md-codeblock",
        Style::Quote => "md-quote",
        Style::Link => "md-link",
        Style::WikiLink => "md-wikilink",
        Style::Embed => "md-embed",
        Style::Tag => "md-tag",
        Style::Task(false) => "md-task",
        Style::Task(true) => "md-task-done",
        Style::Rule => "md-rule",
        Style::Frontmatter => "md-frontmatter",
        Style::TableRow => "md-table",
        Style::TableDelimiter => "md-table-delimiter",
        // Handled by `tag_name`, which never reaches here.
        Style::ListItem(_) => "md-list0",
    }
}

/// Whether a style carries paragraph attributes rather than character ones.
fn is_paragraph_style(style: Style) -> bool {
    matches!(
        style,
        Style::Heading(_)
            | Style::Quote
            | Style::ListItem(_)
            | Style::CodeBlock
            | Style::TableRow
            | Style::TableDelimiter
            | Style::Frontmatter
            | Style::Rule
    )
}

/// The colours a scheme needs. Everything else is weight, scale and family,
/// which do not vary between light and dark.
struct Palette {
    accent: RGBA,
    /// Recessed text: syntax that stays visible, frontmatter, rules.
    dim: RGBA,
    /// Behind inline code and tags.
    surface: RGBA,
    /// Behind a fenced block.
    block: RGBA,
}

impl Palette {
    /// Derive the palette from the view's own text colour.
    ///
    /// Not from a light/dark branch with black and white in it: that is a
    /// guess about the theme, and it is wrong for every theme that is not
    /// Adwaita. The foreground colour is whatever the stylesheet resolved for
    /// this widget, so tinting it produces recessed text and backgrounds that
    /// belong to the theme in use — including a high-contrast one.
    fn from(widget: &impl IsA<gtk::Widget>) -> Self {
        let foreground = widget.as_ref().color();
        let tint = |alpha: f32| {
            RGBA::new(
                foreground.red(),
                foreground.green(),
                foreground.blue(),
                alpha,
            )
        };
        Self {
            accent: adw::StyleManager::default().accent_color_rgba(),
            dim: tint(0.55),
            surface: tint(0.09),
            block: tint(0.05),
        }
    }
}

/// Create every tag the highlighter uses. Call once per buffer.
pub fn install(view: &gtk::TextView) {
    let buffer = view.buffer();
    let table = buffer.tag_table();

    let add = |name: &str, build: &dyn Fn(&gtk::TextTag)| {
        if table.lookup(name).is_some() {
            return;
        }
        let tag = gtk::TextTag::builder().name(name).build();
        build(&tag);
        table.add(&tag);
    };

    // Headings. Scale rather than absolute size, so they follow the font the
    // user chose rather than overriding it.
    add("md-h1", &|tag| {
        tag.set_scale(1.7);
        tag.set_weight(700);
        tag.set_pixels_above_lines(12);
    });
    add("md-h2", &|tag| {
        tag.set_scale(1.4);
        tag.set_weight(700);
        tag.set_pixels_above_lines(10);
    });
    add("md-h3", &|tag| {
        tag.set_scale(1.2);
        tag.set_weight(700);
        tag.set_pixels_above_lines(8);
    });
    add("md-h4", &|tag| {
        tag.set_scale(1.05);
        tag.set_weight(700);
    });

    add("md-bold", &|tag| tag.set_weight(700));
    add("md-italic", &|tag| tag.set_style(gtk::pango::Style::Italic));
    add("md-strike", &|tag| tag.set_strikethrough(true));

    add("md-code", &|tag| {
        tag.set_family(Some("monospace"));
        tag.set_scale(0.95);
    });
    add("md-codeblock", &|tag| {
        tag.set_family(Some("monospace"));
        tag.set_scale(0.95);
    });
    // Monospace, so the pipes in a table line up into columns. A text view
    // cannot draw column rules, so this is the whole of what makes a table
    // read as one.
    add("md-table", &|tag| {
        tag.set_family(Some("monospace"));
        tag.set_scale(0.95);
    });
    add("md-table-delimiter", &|tag| {
        tag.set_family(Some("monospace"));
        tag.set_scale(0.95);
    });

    add("md-quote", &|tag| {
        tag.set_style(gtk::pango::Style::Italic);
        tag.set_left_margin(36);
    });
    // One tag per nesting level, with a hanging indent: the bullet sits in the
    // margin and wrapped lines line up under the item's text instead of under
    // the bullet.
    for level in 0..=crate::model::markdown::MAX_LIST_DEPTH {
        let margin = 36 + i32::from(level) * 24;
        add(&format!("md-list{level}"), &|tag| {
            tag.set_left_margin(margin);
            tag.set_indent(-18);
            tag.set_pixels_above_lines(2);
        });
    }
    add("md-task", &|tag| tag.set_family(Some("monospace")));
    // Dimmed, not struck through: the strikethrough landed on the "[x]" and
    // not on the task, which read as the box being cancelled rather than the
    // job being done.
    add("md-task-done", &|tag| tag.set_family(Some("monospace")));

    add("md-link", &|tag| {
        tag.set_underline(gtk::pango::Underline::Single)
    });
    add("md-wikilink", &|tag| {
        tag.set_underline(gtk::pango::Underline::Single)
    });
    add("md-embed", &|tag| tag.set_style(gtk::pango::Style::Italic));
    add("md-tag", &|_| {});
    add("md-rule", &|_| {});
    add("md-frontmatter", &|tag| {
        tag.set_family(Some("monospace"));
        tag.set_scale(0.85);
    });

    // Applied to syntax characters, invisible until the cursor's block asks
    // for it back.
    add(MARKER, &|tag| tag.set_invisible(true));

    refresh_colours(view);
}

/// Re-derive every colour. Called when the scheme or the accent changes.
///
/// Reading the foreground has to happen after the widget has a style, which is
/// why this takes the view rather than the buffer.
pub fn refresh_colours(view: &gtk::TextView) {
    recolour(&view.buffer(), &Palette::from(view));
}

fn recolour(buffer: &gtk::TextBuffer, palette: &Palette) {
    let table = buffer.tag_table();
    let set = |name: &str, apply: &dyn Fn(&gtk::TextTag)| {
        if let Some(tag) = table.lookup(name) {
            apply(&tag);
        }
    };

    set("md-code", &|tag| {
        tag.set_background_rgba(Some(&palette.surface))
    });
    set("md-codeblock", &|tag| {
        tag.set_paragraph_background_rgba(Some(&palette.block))
    });
    set("md-table", &|tag| {
        tag.set_paragraph_background_rgba(Some(&palette.block))
    });
    set("md-table-delimiter", &|tag| {
        tag.set_paragraph_background_rgba(Some(&palette.block));
        tag.set_foreground_rgba(Some(&palette.dim));
    });
    set("md-quote", &|tag| {
        tag.set_foreground_rgba(Some(&palette.dim))
    });
    set("md-link", &|tag| {
        tag.set_foreground_rgba(Some(&palette.accent))
    });
    set("md-wikilink", &|tag| {
        tag.set_foreground_rgba(Some(&palette.accent))
    });
    set("md-embed", &|tag| {
        tag.set_foreground_rgba(Some(&palette.accent))
    });
    set("md-tag", &|tag| {
        tag.set_foreground_rgba(Some(&palette.accent));
        tag.set_background_rgba(Some(&palette.surface));
    });
    set("md-rule", &|tag| {
        tag.set_foreground_rgba(Some(&palette.dim))
    });
    set("md-frontmatter", &|tag| {
        tag.set_foreground_rgba(Some(&palette.dim));
        tag.set_paragraph_background_rgba(Some(&palette.block));
    });
    set("md-task-done", &|tag| {
        tag.set_foreground_rgba(Some(&palette.dim))
    });
}

/// Remove every tag this module owns from a character range.
pub fn clear(buffer: &gtk::TextBuffer, from: i32, to: i32) {
    let table = buffer.tag_table();
    let start = buffer.iter_at_offset(from);
    let end = buffer.iter_at_offset(to);
    let lists = list_tags();
    let names = STYLE_TAGS
        .iter()
        .map(|name| name.to_string())
        .chain(lists)
        .chain(std::iter::once(MARKER.to_string()));
    for name in names {
        if let Some(tag) = table.lookup(&name) {
            buffer.remove_tag(&tag, &start, &end);
        }
    }
}

/// Apply a scan result to the buffer.
///
/// Offsets are characters, which is what `iter_at_offset` takes — the scanner
/// works in characters for exactly this reason.
pub fn apply(buffer: &gtk::TextBuffer, parsed: &Parsed) {
    let table = buffer.tag_table();
    let length = buffer.char_count();

    let tag_range = |name: &str, start: usize, end: usize| {
        let Some(tag) = table.lookup(name) else {
            return;
        };
        let start = (start as i32).clamp(0, length);
        let end = (end as i32).clamp(0, length);
        if end > start {
            buffer.apply_tag(
                &tag,
                &buffer.iter_at_offset(start),
                &buffer.iter_at_offset(end),
            );
        }
    };

    for Span { start, end, style } in &parsed.spans {
        // `GtkTextView` reads paragraph attributes — left margin, indent,
        // spacing, paragraph background — from the *first character of the
        // line*. A block style whose span begins after its marker, which is
        // every one of them, has to be extended back to the line start or its
        // margins silently do nothing.
        let start = if is_paragraph_style(*style) {
            let mut iter = buffer.iter_at_offset((*start as i32).clamp(0, length));
            iter.set_line_offset(0);
            iter.offset() as usize
        } else {
            *start
        };
        tag_range(&tag_name(*style), start, *end);
    }
    for Marker { start, end, .. } in &parsed.markers {
        tag_range(MARKER, *start, *end);
    }
}

/// Show the syntax of the construct the cursor is in, and hide it everywhere
/// else.
///
/// `cursor` is a character offset, or `None` in reading mode — where nothing is
/// revealed, because there is no caret to reveal it for.
///
/// This is the editing half of the model. The marker tag stays `invisible`
/// globally; what moves is which ranges carry it. Every marker is re-applied
/// each time rather than tracking what was revealed last: it is a handful of
/// ranges, and the bookkeeping version got it wrong in ways that only showed up
/// as stale syntax hours later.
pub fn reveal_markers(buffer: &gtk::TextBuffer, parsed: &Parsed, cursor: Option<usize>) {
    let Some(marker) = buffer.tag_table().lookup(MARKER) else {
        return;
    };

    let length = buffer.char_count();
    buffer.remove_tag(
        &marker,
        &buffer.iter_at_offset(0),
        &buffer.iter_at_offset(length),
    );
    for span in &parsed.markers {
        if cursor.is_some_and(|cursor| span.revealed_by(cursor)) {
            continue;
        }
        let Marker { start, end, .. } = span;
        let start = (*start as i32).clamp(0, length);
        let end = (*end as i32).clamp(0, length);
        if end > start {
            buffer.apply_tag(
                &marker,
                &buffer.iter_at_offset(start),
                &buffer.iter_at_offset(end),
            );
        }
    }
}
