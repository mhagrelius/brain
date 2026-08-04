//! Render the real widget tree to a PNG.
//!
//! Screenshotting a live GNOME Wayland session needs interactive consent, which
//! makes "does this look right?" hard to answer while iterating. This builds the
//! actual widgets against a seeded vault and paints them offscreen instead, so a
//! design change can be looked at in one command.
//!
//! ```sh
//! cargo run --example preview -- /tmp/preview
//! cargo run --example preview -- /tmp/preview dark
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use adw::prelude::*;
use gtk::glib;

use brain::model::note::NoteId;
use brain::model::tree::{self, Listed, Sort};
use brain::model::vault::Vault;
use brain::ui::{BacklinksPanel, BrainWindow, Editor, Hit, Palette, Sidebar, TagTree};

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "/tmp/preview".to_string());
    let dark = args.next().is_some_and(|scheme| scheme == "dark");

    gtk::init().expect("a display — run under xvfb-run if there is none");
    adw::init().expect("libadwaita");

    // An animating widget is a widget that is not finished being laid out.
    // Turning animations off makes a snapshot deterministic rather than a race
    // against a transition.
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_enable_animations(false);
    }

    adw::StyleManager::default().set_color_scheme(if dark {
        adw::ColorScheme::ForceDark
    } else {
        adw::ColorScheme::ForceLight
    });
    if let Some(display) = gtk::gdk::Display::default() {
        brain::ui::load_stylesheet(&display);
    }

    let vault = seeded();
    fs::create_dir_all(&out).expect("output directory");

    let (notes, _) = vault.scan();
    let mut listed: Vec<(NoteId, String)> = notes
        .iter()
        .map(|note| (note.id.clone(), note.excerpt(120)))
        .collect();
    listed.sort_by(|a, b| a.0.cmp(&b.0));

    // Every folder open, since a preview of a collapsed tree says nothing
    // about how a nested one reads.
    let folders = vault.folders();
    let for_tree: Vec<Listed> = listed
        .iter()
        .map(|(id, excerpt)| Listed::new(id.clone(), excerpt.clone()))
        .collect();
    let rows = tree::rows(
        &for_tree,
        &folders,
        &folders.iter().cloned().collect(),
        Sort::Name,
    );

    let sidebar = Sidebar::new();
    sidebar.set_rows(&rows);
    sidebar.select(Some(&NoteId::from_relative("Rust ownership.md")));
    render(
        &sidebar,
        320,
        480,
        &format!("{out}/sidebar-{}.png", scheme(dark)),
    );

    // The same widget showing search results, which is a flat list with the
    // folder spelled out — a different shape worth looking at.
    let results = Sidebar::new();
    results.set_results(&listed);
    render(
        &results,
        320,
        480,
        &format!("{out}/results-{}.png", scheme(dark)),
    );

    // The editor, holding a note that uses every piece of syntax the scanner
    // knows about — so phase 3's styling has something to be judged against.
    //
    // Tall enough for the whole note. A preview of a scrolled slice answers a
    // less useful question than a preview of the note, and `WidgetPaintable`
    // declines to draw a scroller whose content overflows it.
    let editor = Editor::new();
    editor.set_vault_root(Some(vault.root().to_path_buf()));
    let note = vault
        .read(&NoteId::from_relative("Rust ownership.md"))
        .expect("the seeded note");
    editor.load(&note.to_text());
    editor.set_editable(true);
    // Caret inside the emphasis on the first line of prose, so the picture
    // shows what editing looks like: that construct's asterisks are back and
    // nothing else on the line is.
    let body = note.to_text();
    if let Some(at) = body.find("destructive") {
        editor.place_cursor_at(body[..at].chars().count() + 2);
    }
    render(
        &editor,
        640,
        940,
        &format!("{out}/editor-{}.png", scheme(dark)),
    );

    // The same note being read rather than edited: no syntax anywhere, no
    // caret. A second editor rather than a mode switch on the first, because
    // `render` grows the window until the widget draws and a widget that has
    // already been sized skips that, giving a picture of the top of the note.
    let reading = Editor::new();
    reading.set_vault_root(Some(vault.root().to_path_buf()));
    reading.load(&note.to_text());
    reading.set_editable(true);
    reading.set_reading(true);
    render(
        &reading,
        640,
        940,
        &format!("{out}/editor-reading-{}.png", scheme(dark)),
    );

    // The backlinks pane, against the real index rather than made-up rows.
    let index = brain::model::index::Index::build(&notes);
    let backlinks = BacklinksPanel::new();
    backlinks.set_backlinks(
        &index
            .backlinks(&NoteId::from_relative("Borrow checker.md"))
            .iter()
            .map(|backlink| (backlink.from.clone(), backlink.context.clone()))
            .collect::<Vec<_>>(),
    );
    render(
        &backlinks,
        320,
        360,
        &format!("{out}/backlinks-{}.png", scheme(dark)),
    );

    // The tag tree, nested tags and all.
    let tags = TagTree::new();
    tags.set_tags(&index.tags());
    render(&tags, 320, 300, &format!("{out}/tags-{}.png", scheme(dark)));

    // The whole window, assembled. Rendering its content rather than the
    // window itself, because a bare X server has no window manager to map one.
    let window: BrainWindow = glib::Object::new();
    window.set_vault_root(Some(vault.root().to_path_buf()));
    window.set_rows(&rows);
    window.set_tags(&index.tags());
    let opened = NoteId::from_relative("Rust ownership.md");
    let note = vault.read(&opened).expect("the seeded note");
    window.show_note(Some((&opened, &note.to_text())));
    window.set_backlinks(
        &index
            .backlinks(&opened)
            .iter()
            .map(|backlink| (backlink.from.clone(), backlink.context.clone()))
            .collect::<Vec<_>>(),
    );
    window.set_details(
        Some(&opened),
        &["rust".to_string(), "learning".to_string()],
        122,
        Some("2026-07-28".to_string()),
        Some("2026-07-31".to_string()),
    );
    window.set_backlinks_shown(true);
    if let Some(content) = window.content() {
        window.set_content(gtk::Widget::NONE);
        render(
            &content,
            1100,
            760,
            &format!("{out}/window-{}.png", scheme(dark)),
        );
    }

    // The banner, offering the one action its condition has. Rendered on its
    // own because the condition is rare and the shot is the only way to see
    // that the button reads as a way out rather than as a warning about one.
    let diverged: BrainWindow = glib::Object::new();
    diverged.set_rows(&rows);
    diverged.set_tags(&index.tags());
    diverged.show_note(Some((&opened, &note.to_text())));
    diverged.set_banner(
        Some("“Rust ownership” changed on disk — saving will overwrite that"),
        Some("Reload"),
    );
    if let Some(content) = diverged.content() {
        diverged.set_content(gtk::Widget::NONE);
        render(
            &content,
            1100,
            600,
            &format!("{out}/banner-{}.png", scheme(dark)),
        );
    }

    // The window at first start: a vault with nothing in it yet.
    let fresh: BrainWindow = glib::Object::new();
    fresh.set_rows(&[]);
    fresh.set_tags(&[]);
    fresh.show_note(None);
    if let Some(content) = fresh.content() {
        fresh.set_content(gtk::Widget::NONE);
        render(
            &content,
            1100,
            600,
            &format!("{out}/first-run-{}.png", scheme(dark)),
        );
    }

    // The same window collapsed, which is what a narrow screen gets. The
    // breakpoint drives this in the real app; here it is set directly, since
    // an offscreen render has no window to measure.
    let narrow: BrainWindow = glib::Object::new();
    narrow.set_vault_root(Some(vault.root().to_path_buf()));
    narrow.set_rows(&rows);
    narrow.set_tags(&index.tags());
    narrow.show_note(Some((&opened, &note.to_text())));
    narrow.set_collapsed_for_test(true);
    if let Some(content) = narrow.content() {
        narrow.set_content(gtk::Widget::NONE);
        render(
            &content,
            420,
            700,
            &format!("{out}/narrow-{}.png", scheme(dark)),
        );
    }

    // The keyboard shortcuts, which are otherwise undiscoverable.
    let shortcuts = BrainWindow::shortcuts_dialog_for_test();
    if let Some(content) = shortcuts.child() {
        shortcuts.set_child(gtk::Widget::NONE);
        render(
            &content,
            520,
            560,
            &format!("{out}/shortcuts-{}.png", scheme(dark)),
        );
    }

    // The palette, in text mode against real search results.
    let palette = Palette::new();
    let hits: Vec<Hit> = brain::model::search::by_text(&index, "borrow", 10)
        .into_iter()
        .map(|matched| {
            let snippet = matched.snippets.first();
            Hit {
                id: matched.id.as_str().to_string(),
                title: matched.id.title().to_string(),
                detail: snippet.map(|s| s.text.clone()).unwrap_or_default(),
                highlight: snippet.map(|s| (s.start, s.end)),
            }
        })
        .collect();
    palette.set_hits(&hits);
    if let Some(content) = palette.child() {
        palette.set_child(gtk::Widget::NONE);
        render(
            &content,
            640,
            360,
            &format!("{out}/palette-{}.png", scheme(dark)),
        );
    }

    // The empty sidebar, so the empty state is not the only thing untested.
    let empty = Sidebar::new();
    empty.set_rows(&[]);
    render(
        &empty,
        320,
        300,
        &format!("{out}/empty-{}.png", scheme(dark)),
    );

    println!("wrote {out}/*-{}.png", scheme(dark));
}

fn scheme(dark: bool) -> &'static str {
    if dark {
        "dark"
    } else {
        "light"
    }
}

/// Paint a widget offscreen and write it out.
///
/// `WidgetPaintable` declines to draw a scroller whose content overflows it, so
/// a widget that wants more room than it was given produces no render node at
/// all. Rather than hard-coding a height per picture and re-guessing it every
/// time the seed note grows, the window is grown until something is drawn.
fn render(widget: &impl IsA<gtk::Widget>, width: i32, height: i32, path: &str) {
    for factor in [1, 2, 3, 4] {
        if try_render(widget, width, height * factor, path) {
            return;
        }
    }
    eprintln!("{path}: nothing was drawn, even with room to spare");
}

fn try_render(widget: &impl IsA<gtk::Widget>, width: i32, height: i32, path: &str) -> bool {
    let window = gtk::Window::builder()
        .default_width(width)
        .default_height(height)
        .child(widget)
        .build();
    // No titlebar: these are pictures of a widget, and a window decoration
    // around one reads as a mistake.
    window.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    window.present();

    settle();
    let drawn = snapshot(
        &window,
        window.width().max(width),
        window.height().max(height),
        path,
    );

    // Take the widget back before the window goes, so a caller can render the
    // same one twice.
    window.set_child(gtk::Widget::NONE);
    window.destroy();
    drawn
}

/// Run the main loop until there is nothing left to lay out.
///
/// One drain is not enough: presenting a widget schedules work that schedules
/// more, so this pumps until it stops finding any, with a bound so a
/// misbehaving widget cannot hang the run.
fn settle() {
    let context = glib::MainContext::default();
    for _ in 0..100 {
        let mut worked = false;
        while context.iteration(false) {
            worked = true;
        }
        if !worked {
            break;
        }
    }
}

/// Paint a realised window into a PNG. Reports whether anything was drawn.
fn snapshot(window: &impl IsA<gtk::Widget>, width: i32, height: i32, path: &str) -> bool {
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, width as f64, height as f64);

    let Some(node) = snapshot.to_node() else {
        return false;
    };
    let renderer = gtk::gsk::CairoRenderer::new();
    renderer
        .realize(gtk::gdk::Surface::NONE)
        .expect("a renderer");
    let texture = renderer.render_texture(&node, None);
    texture.save_to_png(path).expect("write the png");
    renderer.unrealize();
    true
}

/// A vault with enough in it to show every part of a row.
fn seeded() -> Vault {
    let root: PathBuf = std::env::temp_dir().join("brain-preview");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("vault directory");
    let vault = Vault::new(&root);

    write(
        &vault,
        "Rust ownership.md",
        "---\n\
         tags: [rust, learning]\n\
         aliases: [Ownership]\n\
         ---\n\
         \n\
         # Rust ownership\n\
         \n\
         Moves are **destructive** and borrows are *not*. See [[Borrow checker]]\n\
         for the rules, or [[Borrow checker|the checker]] if you are in a hurry.\n\
         \n\
         ## What it costs\n\
         \n\
         | Operation | Cost     |\n\
         |-----------|----------|\n\
         | Move      | free     |\n\
         | Clone     | `memcpy` |\n\
         \n\
         - [x] Read the chapter\n\
         - [ ] Write it up #rust #project/brain\n\
         \x20\x20- nested one level, which indents\n\
         \x20\x20- and a second that wraps far enough to show the wrapped line \
lining up under the text rather than under the bullet\n\
         \x20\x20\x20\x20- deeper still\n\
         \n\
         > Ownership is the one idea the rest hangs off.\n\
         \n\
         ```rust\n\
         fn take(value: String) -> usize { value.len() }\n\
         ```\n\
         \n\
         ![[diagram.png]]\n\
         \n\
         More at https://doc.rust-lang.org/book/\n",
    );
    write(
        &vault,
        "Borrow checker.md",
        "# Borrow checker\n\nOne mutable borrow, or many shared ones. Back to [[Rust ownership]].\n",
    );
    write(
        &vault,
        "Meetings/Standup.md",
        "Notes from standup: shipping the editor this week. #project/brain\n",
    );
    write(&vault, "Scratch.md", "");

    // A real file behind the embed, so the picture has something to draw.
    let attachments = root.join("attachments");
    fs::create_dir_all(&attachments).expect("attachments directory");
    fs::write(attachments.join("diagram.png"), gradient()).expect("write the image");

    vault
}

/// A small PNG, drawn rather than embedded as bytes so the preview does not
/// carry a binary blob around.
fn gradient() -> Vec<u8> {
    let width = 320;
    let height = 120;
    let mut data = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            data.push((x * 255 / width) as u8);
            data.push((y * 255 / height) as u8);
            data.push(160u8);
        }
    }
    let bytes = glib::Bytes::from_owned(data);
    let texture = gtk::gdk::MemoryTexture::new(
        width,
        height,
        gtk::gdk::MemoryFormat::R8g8b8,
        &bytes,
        (width * 3) as usize,
    );
    texture.save_to_png_bytes().to_vec()
}

fn write(vault: &Vault, path: &str, body: &str) {
    let id = NoteId::from_relative(path);
    if let Some(folder) = Path::new(path).parent() {
        let _ = fs::create_dir_all(vault.root().join(folder));
    }
    vault.create(&id, body).expect("seed a note");
}
