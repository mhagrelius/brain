//! The first run: an empty vault, and the first note ever made in it.
//!
//! Reported from a real install — New Note, a title, confirm, and no file.
//! The difference from `lifecycle.rs` is that this vault starts empty, which
//! is what a first run actually looks like.

use std::cell::RefCell;
use std::fs;
use std::rc::Rc;

use adw::prelude::*;
use brain::model::note::NoteId;
use brain::ui::BrainApplication;

#[test]
fn the_first_note_in_an_empty_vault() {
    let home = tempfile::tempdir().expect("temp dir");
    let vault = home.path().join("Documents");
    fs::create_dir_all(&vault).expect("vault");

    let config_home = home.path().join("config");
    fs::create_dir_all(config_home.join("brain")).expect("config dir");
    fs::write(
        config_home.join("brain/config.json"),
        format!(
            r#"{{"version":1,"vault":{:?},"last_note":null}}"#,
            vault.to_string_lossy()
        ),
    )
    .expect("write config");

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", &config_home);
        std::env::set_var("XDG_DATA_HOME", home.path().join("data"));
    }

    let failure: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let reported = failure.clone();
    let vault_path = vault.clone();

    let app = BrainApplication::with_application_id("us.hagreli.Brain.FirstRun");
    app.connect_activate(move |app| {
        let app = app
            .downcast_ref::<BrainApplication>()
            .expect("our application type")
            .clone();
        let reported = reported.clone();
        let vault_path = vault_path.clone();

        gtk::glib::idle_add_local_once(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let window = app.window().expect("a window");
                assert!(
                    app.listed_notes().is_empty(),
                    "the vault should start empty"
                );

                gtk::prelude::WidgetExt::activate_action(&window, "win.new-note", None)
                    .expect("win.new-note");

                let dialog = find::<adw::AlertDialog>(window.upcast_ref::<gtk::Widget>())
                    .expect("the New Note dialog");
                let entry = find::<adw::EntryRow>(dialog.upcast_ref::<gtk::Widget>())
                    .expect("the title entry");
                entry.set_text("My first note");

                let create = find_button(dialog.upcast_ref::<gtk::Widget>(), "Create")
                    .expect("a Create button");
                create.emit_by_name::<()>("clicked", &[]);

                assert!(
                    vault_path.join("My first note.md").exists(),
                    "no file was written into an empty vault"
                );
                assert_eq!(app.listed_notes().len(), 1, "the note is not in the list");
                assert_eq!(
                    app.open_note_id(),
                    Some(NoteId::from_relative("My first note.md"))
                );
            }));
            if let Err(panic) = result {
                let message = panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "panicked".to_string());
                reported.replace(Some(message));
            }
            app.quit();
        });
    });

    let code = app.run_with_args::<&str>(&[]);
    assert_eq!(code, gtk::glib::ExitCode::SUCCESS);

    let message = failure.borrow().clone();
    if let Some(message) = message {
        panic!("{message}");
    }
}

fn find<T: IsA<gtk::Widget>>(root: &gtk::Widget) -> Option<T> {
    let mut found = None;
    walk(root, &mut |widget| {
        if found.is_none() {
            if let Some(matched) = widget.downcast_ref::<T>() {
                found = Some(matched.clone());
            }
        }
    });
    found
}

fn find_button(root: &gtk::Widget, label: &str) -> Option<gtk::Button> {
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

fn walk(root: &gtk::Widget, visit: &mut impl FnMut(&gtk::Widget)) {
    visit(root);
    let mut child = root.first_child();
    while let Some(widget) = child {
        walk(&widget, visit);
        child = widget.next_sibling();
    }
}
