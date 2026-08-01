//! The real application, driven through a whole session.
//!
//! `widgets.rs` builds widgets and pokes them; this runs `BrainApplication`
//! itself — its startup, its window, its save tick, its vault — against a
//! temporary vault, and checks the files on disk afterwards. It is the test
//! that would catch the pieces being individually right and jointly wrong.
//!
//! One `#[test]`, for the same reason as `widgets.rs`: GTK is thread-affine.
//! The scenario runs inside `activate`, so everything happens on the thread
//! that owns the toolkit, and the application quits when it finishes.

use std::cell::RefCell;
use std::fs;
use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use brain::model::note::NoteId;
use brain::ui::{BrainApplication, Mode};

/// A named step and what it does. Each gets the failure reported against its
/// own name, so a red test says which interaction broke.
type Step = (&'static str, fn(&BrainApplication, &Path));

#[test]
fn a_whole_session() {
    let home = tempfile::tempdir().expect("temp dir");
    let vault = home.path().join("Notes");
    fs::create_dir_all(&vault).expect("vault");

    // A config pointing at the vault, so `activate` does not open the folder
    // chooser — a portal dialog would hang the test.
    let config_home = home.path().join("config");
    fs::create_dir_all(config_home.join("brain")).expect("config dir");
    fs::write(
        config_home.join("brain/config.json"),
        format!(
            r#"{{"version":1,"vault":{:?},"last_note":null,"reading_mode":true}}"#,
            vault.to_string_lossy()
        ),
    )
    .expect("write config");

    // Safety: this is a single-threaded test binary and these are set before
    // any thread that reads them exists.
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", &config_home);
        std::env::set_var("XDG_DATA_HOME", home.path().join("data"));
    }

    seed(&vault);

    let failures: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let collected = failures.clone();
    let vault_path = vault.clone();

    // A test-only id: the real one would make this process a D-Bus remote
    // driving the developer's running app.
    let app = BrainApplication::with_application_id("us.hagreli.Brain.Lifecycle");
    app.connect_activate(move |app| {
        let app = app
            .downcast_ref::<BrainApplication>()
            .expect("our application type")
            .clone();
        let collected = collected.clone();
        let vault_path = vault_path.clone();

        // On an idle, not here: a handler connected to `activate` runs *before*
        // the default one, so the window does not exist yet at this point.
        gtk::glib::idle_add_local_once(move || {
            for (name, step) in STEPS {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    step(&app, &vault_path);
                }));
                if let Err(panic) = result {
                    let message = panic
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_else(|| "panicked".to_string());
                    collected.borrow_mut().push(format!("{name}: {message}"));
                }
            }
            app.quit();
        });
    });

    // No arguments, so cargo's own are not parsed as the application's.
    let code = app.run_with_args::<&str>(&[]);
    assert_eq!(code, gtk::glib::ExitCode::SUCCESS);

    let failures = failures.borrow();
    assert!(failures.is_empty(), "\n  {}", failures.join("\n  "));
}

fn seed(vault: &Path) {
    fs::write(
        vault.join("Rust ownership.md"),
        "---\ntags: [rust]\n---\n\n# Rust ownership\n\nMoves are destructive. See [[Borrow checker]].\n",
    )
    .expect("write");
    fs::write(
        vault.join("Borrow checker.md"),
        "# Borrow checker\n\nOne mutable borrow. #rust\n",
    )
    .expect("write");
    fs::create_dir_all(vault.join("Meetings")).expect("dir");
    fs::write(
        vault.join("Meetings/Standup.md"),
        "Shipping the editor. #project/brain\n",
    )
    .expect("write");
}

/// The alert dialog currently presented over `window`, if any.
fn find_alert_dialog(window: &brain::ui::BrainWindow) -> Option<adw::AlertDialog> {
    let mut found = None;
    walk(window.upcast_ref::<gtk::Widget>(), &mut |widget| {
        if found.is_none() {
            if let Some(dialog) = widget.downcast_ref::<adw::AlertDialog>() {
                found = Some(dialog.clone());
            }
        }
    });
    found
}

fn find_button_labelled(root: &gtk::Widget, label: &str) -> Option<gtk::Button> {
    let mut found = None;
    walk(root, &mut |widget| {
        if found.is_some() {
            return;
        }
        if let Some(button) = widget.downcast_ref::<gtk::Button>() {
            if button.label().is_some_and(|text| text == label) {
                found = Some(button.clone());
            }
        }
    });
    found
}

fn find_entry_row(root: &gtk::Widget) -> Option<adw::EntryRow> {
    let mut found = None;
    walk(root, &mut |widget| {
        if found.is_none() {
            if let Some(entry) = widget.downcast_ref::<adw::EntryRow>() {
                found = Some(entry.clone());
            }
        }
    });
    found
}

fn walk(root: &gtk::Widget, visit: &mut impl FnMut(&gtk::Widget)) {
    visit(root);
    let mut child = root.first_child();
    while let Some(widget) = child {
        walk(&widget, visit);
        child = widget.next_sibling();
    }
}

fn read(vault: &Path, name: &str) -> String {
    fs::read_to_string(vault.join(name)).unwrap_or_else(|_| panic!("{name} should exist"))
}

fn type_into_editor(app: &BrainApplication, text: &str) {
    let window = app.window().expect("a window");
    let editor = window.editor().expect("an editor");
    let at = editor.body().chars().count();
    editor.insert_at_for_test(at, text);
}

const STEPS: &[Step] = &[
    (
        "the window opens in the mode the config remembers",
        |app, _| {
            // Restoring the mode reaches back into the config to record it,
            // and the seeded config here says reading — which is the ordering
            // that would deadlock if the restore held the config open.
            let window = app.window().expect("a window");
            assert!(window.is_reading(), "the remembered mode was not restored");

            // The rest of the session is about editing.
            window.set_reading(false);
        },
    ),
    ("the vault opens with every note listed", |app, _| {
        assert_eq!(app.listed_notes().len(), 3);
    }),
    (
        "a note opens and shows its file, frontmatter included",
        |app, vault| {
            let id = NoteId::from_relative("Rust ownership.md");
            app.open_note(&id);
            assert_eq!(app.open_note_id(), Some(id));

            let window = app.window().expect("a window");
            assert_eq!(
                window.editor_body().unwrap_or_default(),
                read(vault, "Rust ownership.md"),
                "the editor must hold exactly what the file holds"
            );
        },
    ),
    (
        "typing reaches disk when the note is saved",
        |app, vault| {
            type_into_editor(app, "\nA new line typed by the test.\n");
            app.flush_open_note();
            app.save_now();

            let on_disk = read(vault, "Rust ownership.md");
            assert!(
                on_disk.contains("A new line typed by the test."),
                "the edit never reached the file: {on_disk:?}"
            );
            // And the frontmatter it did not touch is untouched.
            assert!(on_disk.starts_with("---\ntags: [rust]\n---\n"));
        },
    ),
    (
        "opening another note saves the one being left",
        |app, vault| {
            type_into_editor(app, "Typed just before switching away.\n");
            app.open_note(&NoteId::from_relative("Borrow checker.md"));

            assert!(
                read(vault, "Rust ownership.md").contains("Typed just before switching away."),
                "switching notes dropped unsaved typing"
            );
            assert_eq!(
                app.open_note_id(),
                Some(NoteId::from_relative("Borrow checker.md"))
            );
        },
    ),
    ("a new note is created, opened and empty", |app, vault| {
        app.create_note("Ownership rules");
        assert_eq!(
            app.open_note_id(),
            Some(NoteId::from_relative("Ownership rules.md"))
        );
        assert_eq!(read(vault, "Ownership rules.md"), "");
        assert_eq!(app.listed_notes().len(), 4);
    }),
    (
        "a link completion offers real notes and never the open one",
        |app, _| {
            let candidates = app.link_candidates("bor");
            assert!(
                candidates.iter().any(|c| c == "Borrow checker"),
                "expected Borrow checker in {candidates:?}"
            );
            // "Ownership rules" is open, so it must not offer to link to itself.
            assert!(!candidates.iter().any(|c| c == "Ownership rules"));
        },
    ),
    ("following a link opens the note it names", |app, _| {
        app.follow_link("Borrow checker");
        assert_eq!(
            app.open_note_id(),
            Some(NoteId::from_relative("Borrow checker.md"))
        );
    }),
    ("backlinks list the notes pointing here", |app, _| {
        let backlinks = app.backlinks_of_open_note();
        assert_eq!(backlinks.len(), 1, "{backlinks:?}");
        assert_eq!(backlinks[0].0, NoteId::from_relative("Rust ownership.md"));
        assert!(backlinks[0].1.contains("Borrow checker"));
    }),
    (
        "renaming repoints every link that pointed at the note",
        |app, vault| {
            app.open_note(&NoteId::from_relative("Borrow checker.md"));
            app.rename_note("Borrowing");

            assert!(vault.join("Borrowing.md").exists());
            assert!(!vault.join("Borrow checker.md").exists());
            assert!(
                read(vault, "Rust ownership.md").contains("[[Borrowing]]"),
                "the inbound link was not repointed"
            );
            assert_eq!(
                app.open_note_id(),
                Some(NoteId::from_relative("Borrowing.md"))
            );
        },
    ),
    ("tags are collected from frontmatter and body", |app, _| {
        let tags: Vec<String> = app.tags().into_iter().map(|(tag, _)| tag).collect();
        for expected in ["rust", "project", "project/brain"] {
            assert!(
                tags.iter().any(|t| t == expected),
                "{expected} missing from {tags:?}"
            );
        }
    }),
    (
        "filtering by a tag narrows the list and clearing restores it",
        |app, _| {
            let all = app.listed_notes().len();
            app.filter_by_tag(Some("project/brain"));
            assert_eq!(app.listed_notes().len(), 1);
            app.filter_by_tag(None);
            assert_eq!(app.listed_notes().len(), all);
        },
    ),
    (
        "filtering by a tag nothing carries clears itself",
        |app, _| {
            let all = app.listed_notes().len();
            app.filter_by_tag(Some("nothing-has-this"));
            assert_eq!(
                app.listed_notes().len(),
                all,
                "a filter matching nothing must not leave an empty list with no way out"
            );
        },
    ),
    ("dropping a file attaches it and embeds it", |app, vault| {
        app.open_note(&NoteId::from_relative("Ownership rules.md"));

        let dropped = vault.parent().expect("parent").join("diagram.png");
        fs::write(&dropped, PNG).expect("write");
        app.attach_files(&[dropped.to_string_lossy().to_string()]);
        app.flush_open_note();
        app.save_now();

        assert!(vault.join("attachments/diagram.png").exists());
        assert!(
            read(vault, "Ownership rules.md").contains("![[diagram.png]]"),
            "the embed was not written into the note"
        );
    }),
    ("search finds notes by title and by text", |app, _| {
        let by_title = app.search("borrow", Mode::Title);
        assert!(
            by_title.iter().any(|hit| hit.title == "Borrowing"),
            "{by_title:?}"
        );

        let by_text = app.search("mutable", Mode::Text);
        assert!(
            by_text.iter().any(|hit| hit.id == "Borrowing.md"),
            "{by_text:?}"
        );
        let hit = by_text
            .iter()
            .find(|hit| hit.id == "Borrowing.md")
            .expect("a hit");
        let (start, end) = hit.highlight.expect("a highlighted range");
        let matched: String = hit.detail.chars().skip(start).take(end - start).collect();
        assert_eq!(matched.to_lowercase(), "mutable");
    }),
    (
        "deleting removes the file and leaves other notes alone",
        |app, vault| {
            app.open_note(&NoteId::from_relative("Ownership rules.md"));
            let before = app.listed_notes().len();
            app.delete_open_note();

            assert!(!vault.join("Ownership rules.md").exists());
            assert_eq!(app.listed_notes().len(), before - 1);
            assert_eq!(app.open_note_id(), None);
            // Deleting a note must not edit any other note.
            assert!(read(vault, "Rust ownership.md").contains("[[Borrowing]]"));
        },
    ),
    (
        "a note changed on disk is picked up on reload",
        |app, vault| {
            app.open_note(&NoteId::from_relative("Rust ownership.md"));
            let path = vault.join("Rust ownership.md");
            let changed = format!(
                "{}\nAdded by something else.\n",
                read(vault, "Rust ownership.md")
            );
            fs::write(&path, &changed).expect("write");

            app.reload_vault();
            let window = app.window().expect("a window");
            assert_eq!(
                window.editor_body().unwrap_or_default(),
                changed,
                "an external edit was not picked up"
            );
        },
    ),
    (
        "a note deleted on disk does not take the app with it",
        |app, vault| {
            fs::remove_file(vault.join("Meetings/Standup.md")).expect("remove");
            app.reload_vault();
            assert!(app.open_note_id().is_some(), "the open note should survive");
        },
    ),
    (
        "the New Note dialog actually creates the note",
        |app, vault| {
            // Reported from a real install: typing a title and confirming did
            // nothing at all.
            let window = app.window().expect("a window");
            gtk::prelude::WidgetExt::activate_action(&window, "win.new-note", None)
                .expect("win.new-note");

            let dialog = find_alert_dialog(&window).expect("the New Note dialog");
            let entry =
                find_entry_row(dialog.upcast_ref::<gtk::Widget>()).expect("the title entry");
            entry.set_text("Made by the dialog");
            // The real button, not the signal it emits: this is the path a
            // click takes, and the reported failure was on that path.
            find_button_labelled(dialog.upcast_ref::<gtk::Widget>(), "Create")
                .expect("a Create button")
                .emit_by_name::<()>("clicked", &[]);

            assert!(
                vault.join("Made by the dialog.md").exists(),
                "confirming the dialog did not create the note"
            );
            assert_eq!(
                app.open_note_id(),
                Some(NoteId::from_relative("Made by the dialog.md")),
                "the new note was not opened"
            );
            assert!(
                find_alert_dialog(&window).is_none(),
                "the dialog is still on screen after confirming"
            );
            app.delete_open_note();
        },
    ),
    (
        "pressing Enter in the title entry creates the note too",
        |app, vault| {
            let window = app.window().expect("a window");
            gtk::prelude::WidgetExt::activate_action(&window, "win.new-note", None)
                .expect("win.new-note");

            let dialog = find_alert_dialog(&window).expect("the New Note dialog");
            let entry =
                find_entry_row(dialog.upcast_ref::<gtk::Widget>()).expect("the title entry");
            entry.set_text("Made by pressing Enter");
            entry.emit_by_name::<()>("entry-activated", &[]);

            assert!(
                vault.join("Made by pressing Enter.md").exists(),
                "Enter in the entry did not create the note"
            );
            assert!(
                find_alert_dialog(&window).is_none(),
                "the dialog is still on screen after confirming with Enter"
            );
            app.delete_open_note();
        },
    ),
    (
        "every window action is wired to something that runs",
        |app, _| {
            // The path a menu item or a keyboard shortcut takes. An action
            // named in a menu but never added to the group does nothing at
            // all, and nothing warns about it.
            let window = app.window().expect("a window");
            app.open_note(&NoteId::from_relative("Rust ownership.md"));

            for name in [
                "new-note",
                "toggle-sidebar",
                "toggle-backlinks",
                "toggle-reading",
                "clear-filter",
                "reload",
                "save",
                "quick-open",
                "search-text",
                "shortcuts",
                "unused-attachments",
                "rename-note",
                "delete-note",
                "choose-vault",
            ] {
                let full = format!("win.{name}");
                assert!(
                    window.has_action(name),
                    "{full} is named in the UI but not registered"
                );
            }

            // The menu also names application-level actions.
            for name in ["quit", "about"] {
                assert!(
                    gtk::gio::prelude::ActionGroupExt::has_action(app, name),
                    "app.{name} is named in the UI but not registered"
                );
            }

            // The ones with no dialog behind them are safe to actually run,
            // and running them is the only way to know they are connected.
            for name in [
                "toggle-sidebar",
                "toggle-backlinks",
                "toggle-reading",
                "clear-filter",
                "reload",
                "save",
                "toggle-sidebar",
                "toggle-backlinks",
                "toggle-reading",
            ] {
                gtk::prelude::WidgetExt::activate_action(&window, &format!("win.{name}"), None)
                    .unwrap_or_else(|_| panic!("win.{name} refused to activate"));
            }
        },
    ),
    (
        "an unrealised window does not overwrite a remembered size",
        |app, _| {
            // The size is recorded twice, and the second call happens during
            // shutdown when the window may report 0x0. Writing that would
            // reopen the app at its minimum size.
            let window = app.window().expect("a window");
            window.set_default_size(1000, 700);
            app.remember_window();
            let after_real = app.remembered_size();

            gtk::prelude::WidgetExt::unrealize(&window);
            app.remember_window();

            assert_eq!(
                app.remembered_size(),
                after_real,
                "a zero-sized window overwrote the remembered size"
            );
            assert!(
                app.remembered_size().map_or(true, |(w, h)| w > 0 && h > 0),
                "a zero size was recorded"
            );
        },
    ),
    (
        "the mode the window is in outlives the session",
        |app, _| {
            // Reading is a choice about how you work, not about one note; a
            // mode the app forgets is one you re-choose every launch.
            let window = app.window().expect("a window");
            gtk::prelude::WidgetExt::activate_action(&window, "win.toggle-reading", None)
                .expect("win.toggle-reading");
            assert!(window.is_reading());
            assert!(app.remembered_reading(), "reading mode was not recorded");

            gtk::prelude::WidgetExt::activate_action(&window, "win.toggle-reading", None)
                .expect("win.toggle-reading");
            assert!(!window.is_reading());
            assert!(!app.remembered_reading());
        },
    ),
    ("the vault can be pointed somewhere else", |app, vault| {
        let second = vault.parent().expect("parent").join("Second vault");
        fs::create_dir_all(&second).expect("dir");
        fs::write(second.join("Only note.md"), "Alone.\n").expect("write");

        app.set_vault(&second);
        assert_eq!(app.listed_notes().len(), 1);
        assert_eq!(app.open_note_id(), None);

        // And back, with the notes that survived the earlier steps.
        app.set_vault(vault);
        let titles: Vec<String> = app
            .listed_notes()
            .into_iter()
            .map(|(id, _)| id.title().to_string())
            .collect();
        assert_eq!(titles, ["Borrowing", "Rust ownership"], "{titles:?}");
    }),
];

/// A one-pixel PNG, so an attachment is a real image.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
    0x42, 0x60, 0x82,
];
