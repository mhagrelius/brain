//! Whole scenarios against a real vault, with no display anywhere near them.
//!
//! These are the cases that used to be reachable only through the GTK harness,
//! because the rules lived inside a `GObject`. They are the reason the notebook
//! was lifted out: a plain `#[test]` per scenario, no `Xvfb`, no thread
//! affinity, and no single `STEPS` table that every case has to queue behind.
//!
//! `tests/session.rs` in the shell covers the vault and the index directly.
//! This covers what the *application* does with them.

use std::path::Path;

use brain_core::note::NoteId;
use brain_core::notebook::{Alert, External, Failed, Moved, Notebook, Renamed, Saved};
use brain_core::vault::VaultError;

/// A notebook over a fresh vault seeded with `notes`, as (relative path, text).
fn notebook(notes: &[(&str, &str)]) -> (tempfile::TempDir, Notebook) {
    let directory = tempfile::tempdir().expect("temp dir");
    for (path, text) in notes {
        let full = directory.path().join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(&full, text).expect("write note");
    }
    let mut notebook = Notebook::default();
    notebook.set_vault(directory.path());
    (directory, notebook)
}

fn read(vault: &Path, name: &str) -> String {
    std::fs::read_to_string(vault.join(name)).expect("read note")
}

fn id(path: &str) -> NoteId {
    NoteId::from_relative(path)
}

#[test]
fn a_vault_opens_with_every_note_listed() {
    let (_dir, notebook) = notebook(&[
        ("Rust ownership.md", "Moves are destructive."),
        ("Meetings/standup.md", "Hold the release."),
    ]);

    let listed: Vec<String> = notebook
        .listed_notes()
        .into_iter()
        .map(|(id, _)| id.as_str().to_string())
        .collect();
    assert_eq!(listed, vec!["Meetings/standup.md", "Rust ownership.md"]);
}

#[test]
fn a_new_note_is_created_in_a_folder_and_opened() {
    let (dir, mut notebook) = notebook(&[]);
    notebook
        .create_folder("", "Meetings")
        .expect("create folder");

    let id = notebook
        .create_note_in("Meetings", "Standup")
        .expect("create note");

    assert_eq!(id.as_str(), "Meetings/Standup.md");
    assert_eq!(notebook.open_note_id(), Some(id));
    assert_eq!(read(dir.path(), "Meetings/Standup.md"), "");
}

#[test]
fn a_second_note_of_the_same_name_is_suffixed_rather_than_clobbering() {
    let (_dir, mut notebook) = notebook(&[("Standup.md", "the first one")]);

    let id = notebook.create_note_in("", "Standup").expect("create note");

    assert_eq!(id.as_str(), "Standup 2.md");
}

#[test]
fn editing_and_saving_writes_the_file_and_updates_the_index() {
    let (dir, mut notebook) = notebook(&[("Note.md", "before")]);
    notebook.load_note(&id("Note.md")).expect("open");

    notebook.flush("after, with #atag");
    assert!(notebook.is_dirty(), "a changed body did not mark dirty");

    assert!(matches!(notebook.save_now(), Saved::Written));
    assert_eq!(read(dir.path(), "Note.md"), "after, with #atag");
    assert!(!notebook.is_dirty());

    // The index saw it without a rescan: the tag is already searchable.
    assert_eq!(notebook.tags(), vec![("atag".to_string(), 1)]);
}

#[test]
fn saving_an_unchanged_note_writes_nothing() {
    let (_dir, mut notebook) = notebook(&[("Note.md", "text")]);
    notebook.load_note(&id("Note.md")).expect("open");

    notebook.flush("text");
    assert!(matches!(notebook.save_now(), Saved::Clean));
}

#[test]
fn renaming_rewrites_every_inbound_link_and_says_how_many() {
    let (dir, mut notebook) = notebook(&[
        ("Ownership.md", "the note itself"),
        ("A.md", "see [[Ownership]] for this"),
        ("B.md", "also [[Ownership]]"),
        ("C.md", "unrelated"),
    ]);
    notebook.load_note(&id("Ownership.md")).expect("open");

    let outcome = notebook.rename_note("Borrowing");

    let Renamed::Done { to, links } = outcome else {
        panic!("expected a rename, got {outcome:?}");
    };
    assert_eq!(to.as_str(), "Borrowing.md");
    assert_eq!(links, 2);
    assert!(read(dir.path(), "A.md").contains("[[Borrowing]]"));
    assert!(read(dir.path(), "B.md").contains("[[Borrowing]]"));
    assert_eq!(read(dir.path(), "C.md"), "unrelated");
    // The editor followed the file.
    assert_eq!(
        notebook.open_note_id().map(|id| id.as_str().to_string()),
        Some("Borrowing.md".to_string())
    );
}

#[test]
fn renaming_to_the_same_title_does_nothing() {
    let (_dir, mut notebook) = notebook(&[("Note.md", "text")]);
    notebook.load_note(&id("Note.md")).expect("open");

    assert!(matches!(notebook.rename_note("Note"), Renamed::Unchanged));
}

#[test]
fn moving_a_note_leaves_its_inbound_links_alone() {
    let (dir, mut notebook) =
        notebook(&[("Ownership.md", "the note"), ("A.md", "see [[Ownership]]")]);
    notebook.create_folder("", "Rust").expect("create folder");

    let outcome = notebook.move_note(&id("Ownership.md"), "Rust");

    let Moved::Done { to, destination } = outcome else {
        panic!("expected a move, got {outcome:?}");
    };
    assert_eq!(to.as_str(), "Rust/Ownership.md");
    assert_eq!(destination, "Rust");
    // Links resolve by title and the title did not change, so nothing was
    // rewritten — the whole reason a move is cheap.
    assert_eq!(read(dir.path(), "A.md"), "see [[Ownership]]");
}

#[test]
fn moving_a_note_to_where_it_already_is_does_nothing() {
    let (_dir, mut notebook) = notebook(&[("Rust/Ownership.md", "the note")]);

    assert!(matches!(
        notebook.move_note(&id("Rust/Ownership.md"), "Rust"),
        Moved::Unchanged
    ));
}

#[test]
fn deleting_the_open_note_clears_it_and_forgets_it() {
    let (dir, mut notebook) = notebook(&[("Note.md", "text")]);
    notebook.load_note(&id("Note.md")).expect("open");

    let deleted = notebook.delete_open_note().expect("delete");

    assert_eq!(deleted.as_str(), "Note.md");
    assert_eq!(notebook.open_note_id(), None);
    assert!(!dir.path().join("Note.md").exists());
    assert!(notebook.listed_notes().is_empty());
    // The pending write was dropped, or a later save would recreate the file.
    assert!(!notebook.is_dirty());
}

#[test]
fn a_folder_is_deleted_only_when_empty() {
    let (dir, mut notebook) = notebook(&[("Meetings/standup.md", "text")]);

    let outcome = notebook.delete_folder("Meetings");
    assert!(
        matches!(outcome, Err(Failed::Vault(VaultError::FolderNotEmpty(_)))),
        "a folder holding a note was deleted: {outcome:?}"
    );

    // The file manager is a better place to mean "and everything in it", so
    // emptying it is the only way through.
    std::fs::remove_file(dir.path().join("Meetings/standup.md")).expect("remove");
    assert!(notebook.delete_folder("Meetings").is_ok());
}

#[test]
fn renaming_a_folder_takes_its_notes_with_it() {
    let (dir, mut notebook) = notebook(&[("Meetings/standup.md", "text")]);
    notebook
        .load_note(&id("Meetings/standup.md"))
        .expect("open");

    notebook
        .rename_folder("Meetings", "Standups")
        .expect("rename");

    assert!(dir.path().join("Standups/standup.md").exists());
    assert!(!dir.path().join("Meetings").exists());
    // The open note followed the folder rather than being left pointing at a
    // path that no longer exists.
    assert_eq!(
        notebook.open_note_id().map(|id| id.as_str().to_string()),
        Some("Standups/standup.md".to_string())
    );
}

#[test]
fn a_folder_cannot_be_moved_inside_itself() {
    let (_dir, mut notebook) = notebook(&[("A/B/note.md", "text")]);

    let outcome = notebook.relocate_folder("A", "A/B/A");
    assert!(
        matches!(outcome, Err(Failed::Vault(VaultError::IntoItself(_)))),
        "a folder was moved into itself: {outcome:?}"
    );
}

#[test]
fn an_operation_without_a_vault_says_so_rather_than_naming_a_path() {
    let mut notebook = Notebook::default();

    assert!(matches!(
        notebook.create_note_in("", "Note"),
        Err(Failed::NoVault)
    ));
    assert_eq!(Failed::NoVault.to_string(), "no vault is open");
}

// ---- what the watcher brings in ----

#[test]
fn an_external_change_to_an_unedited_open_note_is_taken_on_silently() {
    let (dir, mut notebook) = notebook(&[("Note.md", "before")]);
    notebook.load_note(&id("Note.md")).expect("open");

    std::fs::write(dir.path().join("Note.md"), "after").expect("write");

    assert!(matches!(
        notebook.absorb_external_changes(),
        External::Reloaded
    ));
    assert_eq!(
        notebook.open_note_text().map(|(_, text)| text),
        Some("after".to_string())
    );
}

#[test]
fn an_external_change_beside_unsaved_edits_diverges_rather_than_choosing() {
    let (dir, mut notebook) = notebook(&[("Note.md", "before")]);
    notebook.load_note(&id("Note.md")).expect("open");
    notebook.flush("what I typed");

    std::fs::write(dir.path().join("Note.md"), "what they typed").expect("write");

    let outcome = notebook.absorb_external_changes();
    let External::Diverged {
        id: diverged,
        on_disk,
    } = outcome
    else {
        panic!("expected a divergence, got {outcome:?}");
    };
    assert_eq!(diverged.as_str(), "Note.md");
    assert_eq!(on_disk, "what they typed");
    // Neither side was thrown away: the editor still holds what was typed.
    assert_eq!(
        notebook.open_note_text().map(|(_, text)| text),
        Some("what I typed".to_string())
    );
    assert_eq!(read(dir.path(), "Note.md"), "what they typed");
}

#[test]
fn taking_the_disk_version_drops_the_local_edits() {
    let (dir, mut notebook) = notebook(&[("Note.md", "before")]);
    notebook.load_note(&id("Note.md")).expect("open");
    notebook.flush("what I typed");
    std::fs::write(dir.path().join("Note.md"), "what they typed").expect("write");
    notebook.absorb_external_changes();

    assert!(notebook.take_disk_version());

    assert_eq!(
        notebook.open_note_text().map(|(_, text)| text),
        Some("what they typed".to_string())
    );
    assert!(!notebook.is_dirty(), "the resolved note was still dirty");
}

#[test]
fn brains_own_save_is_not_reported_as_an_external_change() {
    let (_dir, mut notebook) = notebook(&[("Note.md", "before")]);
    notebook.load_note(&id("Note.md")).expect("open");
    notebook.flush("after");
    notebook.save_now();

    // The watcher fires for Brain's own write as loudly as for anyone else's.
    assert!(matches!(
        notebook.absorb_external_changes(),
        External::Quiet
    ));
}

#[test]
fn a_note_deleted_outside_brain_stays_open_and_says_so() {
    let (dir, mut notebook) = notebook(&[("Note.md", "text")]);
    notebook.load_note(&id("Note.md")).expect("open");

    std::fs::remove_file(dir.path().join("Note.md")).expect("remove");

    let outcome = notebook.absorb_external_changes();
    let External::Vanished { id: gone } = outcome else {
        panic!("expected a vanishing, got {outcome:?}");
    };
    assert_eq!(gone.as_str(), "Note.md");
    // Still open here: losing what is on screen because a file moved is worse
    // than showing a note that no longer has a file.
    assert!(notebook.open_note_text().is_some());
}

// ---- what the banner says, when several things are wrong ----

#[test]
fn a_divergence_puts_a_reload_offer_on_the_banner() {
    let (dir, mut notebook) = notebook(&[("Note.md", "before")]);
    notebook.load_note(&id("Note.md")).expect("open");
    notebook.flush("what I typed");
    std::fs::write(dir.path().join("Note.md"), "what they typed").expect("write");

    notebook.absorb_external_changes();

    assert_eq!(notebook.alert(), Some(Alert::Diverged(id("Note.md"))));
}

#[test]
fn resolving_a_divergence_takes_the_disk_version_and_clears_the_banner() {
    let (dir, mut notebook) = notebook(&[("Note.md", "before")]);
    notebook.load_note(&id("Note.md")).expect("open");
    notebook.flush("what I typed");
    std::fs::write(dir.path().join("Note.md"), "what they typed").expect("write");
    notebook.absorb_external_changes();

    assert!(notebook.resolve_alert());

    assert_eq!(
        notebook.open_note_text().map(|(_, text)| text),
        Some("what they typed".to_string())
    );
    assert_eq!(notebook.alert(), None);
}

#[test]
fn saving_through_a_divergence_keeps_your_version_and_clears_the_banner() {
    let (dir, mut notebook) = notebook(&[("Note.md", "before")]);
    notebook.load_note(&id("Note.md")).expect("open");
    notebook.flush("what I typed");
    std::fs::write(dir.path().join("Note.md"), "what they typed").expect("write");
    notebook.absorb_external_changes();

    // Doing nothing is how you keep your own edits, and the next save is what
    // "nothing" turns into.
    assert!(matches!(notebook.save_now(), Saved::Written));

    assert_eq!(read(dir.path(), "Note.md"), "what I typed");
    assert_eq!(notebook.alert(), None);
}

#[test]
fn restoring_a_vanished_note_writes_it_back() {
    let (dir, mut notebook) = notebook(&[("Note.md", "the only copy")]);
    notebook.load_note(&id("Note.md")).expect("open");
    std::fs::remove_file(dir.path().join("Note.md")).expect("remove");
    notebook.absorb_external_changes();

    assert_eq!(notebook.alert(), Some(Alert::Vanished(id("Note.md"))));
    assert!(notebook.resolve_alert());

    assert_eq!(read(dir.path(), "Note.md"), "the only copy");
    assert_eq!(notebook.alert(), None);
    // And it is back in the index, not just back on disk.
    assert_eq!(notebook.listed_notes().len(), 1);
}

#[test]
fn a_vanished_note_that_comes_back_clears_itself() {
    let (dir, mut notebook) = notebook(&[("Note.md", "text")]);
    notebook.load_note(&id("Note.md")).expect("open");
    std::fs::remove_file(dir.path().join("Note.md")).expect("remove");
    notebook.absorb_external_changes();
    assert!(notebook.alert().is_some());

    // Restored from the trash, or a `git checkout` that undid the delete.
    std::fs::write(dir.path().join("Note.md"), "text").expect("write");
    notebook.absorb_external_changes();

    assert_eq!(notebook.alert(), None);
}

#[test]
fn opening_another_note_clears_the_previous_one_s_banner() {
    let (dir, mut notebook) = notebook(&[("A.md", "before"), ("B.md", "other")]);
    notebook.load_note(&id("A.md")).expect("open");
    notebook.flush("what I typed");
    std::fs::write(dir.path().join("A.md"), "what they typed").expect("write");
    notebook.absorb_external_changes();
    assert!(notebook.alert().is_some());

    notebook.load_note(&id("B.md")).expect("open");

    assert_eq!(notebook.alert(), None);
}

#[cfg(unix)]
#[test]
fn a_save_failure_outranks_a_divergence() {
    use std::os::unix::fs::PermissionsExt;

    let (dir, mut notebook) = notebook(&[("Note.md", "before")]);
    notebook.load_note(&id("Note.md")).expect("open");
    notebook.flush("what I typed");
    std::fs::write(dir.path().join("Note.md"), "what they typed").expect("write");
    notebook.absorb_external_changes();
    assert!(matches!(notebook.alert(), Some(Alert::Diverged(_))));

    // Writing now fails: the vault cannot take the temporary file.
    let readable = std::fs::Permissions::from_mode(0o555);
    std::fs::set_permissions(dir.path(), readable).expect("chmod");
    let outcome = notebook.save_now();

    assert!(matches!(outcome, Saved::Failed(_)), "the write succeeded");
    // Work is being lost *now*; in a divergence both versions are safe. A save
    // failure hidden behind a divergence would be a lost note.
    assert!(
        matches!(notebook.alert(), Some(Alert::NotSaving(_))),
        "the divergence was still on the banner: {:?}",
        notebook.alert()
    );

    // Writable again, or the temporary directory cannot be cleaned up.
    let writable = std::fs::Permissions::from_mode(0o755);
    std::fs::set_permissions(dir.path(), writable).expect("chmod back");
}

// ---- what the sidebar asks for ----

#[test]
fn a_tag_filter_narrows_the_list_and_an_unknown_tag_clears_itself() {
    let (_dir, mut notebook) = notebook(&[
        ("A.md", "about #rust"),
        ("B.md", "about #python"),
        ("C.md", "about nothing"),
    ]);

    assert!(notebook.filter_by_tag(Some("rust")));
    let listed: Vec<String> = notebook
        .listed_notes()
        .into_iter()
        .map(|(id, _)| id.as_str().to_string())
        .collect();
    assert_eq!(listed, vec!["A.md"]);

    // A tag nothing carries clears the filter rather than showing an empty
    // list with no way out of it.
    assert!(!notebook.filter_by_tag(Some("haskell")));
    assert_eq!(notebook.listed_notes().len(), 3);
}

#[test]
fn the_sidebar_search_puts_title_matches_before_text_matches() {
    let (_dir, mut notebook) = notebook(&[
        ("Ownership.md", "nothing relevant here"),
        ("Other.md", "a note about ownership"),
    ]);

    assert!(notebook.set_query("ownership"));
    let results: Vec<String> = notebook
        .search_results()
        .into_iter()
        .map(|(id, _)| id.as_str().to_string())
        .collect();
    assert_eq!(results, vec!["Ownership.md", "Other.md"]);

    // Setting the same query again is not a change, so the shell can skip the
    // rebuild.
    assert!(!notebook.set_query("ownership"));
}

#[test]
fn a_new_note_lands_in_the_folder_last_worked_in() {
    let (_dir, mut notebook) = notebook(&[]);
    notebook
        .create_folder("", "Meetings")
        .expect("create folder");

    // Creating the folder is enough to aim at it; so is opening a note in one.
    assert_eq!(notebook.current_folder(), "Meetings");

    let id = notebook
        .create_note_in(&notebook.current_folder(), "Standup")
        .expect("create");
    assert_eq!(id.as_str(), "Meetings/Standup.md");
}

#[test]
fn half_a_shared_vector_store_configuration_is_none_at_all() {
    let (_dir, mut notebook) = notebook(&[]);

    assert_eq!(notebook.shared_vectors(), None);

    // A URL with no token cannot authenticate and a token with no URL has
    // nowhere to go. Either alone is a mistake nobody would see a message
    // about, so neither is half-honoured.
    notebook.config_mut().vectors_url = Some("http://nas:8082".into());
    assert_eq!(notebook.shared_vectors(), None);

    notebook.config_mut().vectors_token = Some("  ".into());
    assert_eq!(notebook.shared_vectors(), None);

    notebook.config_mut().vectors_token = Some("a-token".into());
    assert_eq!(
        notebook.shared_vectors(),
        Some(("http://nas:8082".to_string(), "a-token".to_string()))
    );
}

#[test]
fn a_link_to_a_missing_note_resolves_to_missing() {
    use brain_core::index::Resolution;
    let (_dir, notebook) = notebook(&[("A.md", "see [[Nowhere]]")]);

    assert!(matches!(
        notebook.resolve_link("Nowhere"),
        Resolution::Missing
    ));
}

#[test]
fn a_note_never_offers_itself_as_a_link_candidate() {
    let (_dir, mut notebook) =
        notebook(&[("Ownership.md", "text"), ("Ownership notes.md", "text")]);
    notebook.load_note(&id("Ownership.md")).expect("open");

    let candidates = notebook.link_candidates("Owner");

    assert!(
        !candidates.contains(&"Ownership".to_string()),
        "the open note offered itself: {candidates:?}"
    );
    assert!(candidates.contains(&"Ownership notes".to_string()));
}
