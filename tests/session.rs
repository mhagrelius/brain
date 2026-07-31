//! Whole scenarios against a real vault in a temporary directory.
//!
//! No GTK, no display. These are the tests that would catch a change breaking
//! the way the pieces fit together, as opposed to the unit tests inside each
//! module which catch a change breaking one of them.

use std::fs;

use brain::model::index::{Index, Resolution};
use brain::model::markdown;
use brain::model::note::NoteId;
use brain::model::vault::Vault;

fn vault() -> (tempfile::TempDir, Vault) {
    let directory = tempfile::tempdir().expect("temp dir");
    let vault = Vault::new(directory.path());
    (directory, vault)
}

fn id(path: &str) -> NoteId {
    NoteId::from_relative(path)
}

/// Open a vault the way the application will: scan, then index.
fn open(vault: &Vault) -> Index {
    let (notes, problems) = vault.scan();
    assert!(problems.is_empty(), "{problems:?}");
    Index::build(&notes)
}

/// Rename a note and repoint every link that pointed at it — the whole
/// operation, as the application will perform it.
fn rename(vault: &Vault, index: &mut Index, from: &NoteId, to: &NoteId) {
    let inbound: Vec<NoteId> = index
        .backlinks(from)
        .iter()
        .map(|backlink| backlink.from.clone())
        .collect();

    vault.rename(from, to).expect("rename");
    index.rename(from, to);

    for id in inbound {
        let mut note = vault.read(&id).expect("read");
        let Some(body) = markdown::rewrite_target(&note.body, from.title(), to.title()) else {
            continue;
        };
        note.body = body;
        vault.write(&note).expect("write");
        index.update(&note);
    }
}

#[test]
fn a_vault_survives_a_full_round_trip_through_disk() {
    let (_directory, vault) = vault();
    let source = "---\ntags: [rust, learning]\naliases: [Ownership]\n---\n\n\
                  # Rust ownership\n\nMoves are **destructive**. See [[Borrow checker]].\n";
    vault
        .create(&id("Rust ownership.md"), source)
        .expect("create");
    vault
        .create(&id("Borrow checker.md"), "# Borrow checker\n")
        .expect("create");

    let index = open(&vault);
    assert_eq!(index.len(), 2);

    // Reading and writing without editing must not touch a byte, or every save
    // shows up in git as a change nobody made.
    let note = vault.read(&id("Rust ownership.md")).expect("read");
    vault.write(&note).expect("write");
    assert_eq!(
        fs::read_to_string(vault.path_of(&id("Rust ownership.md"))).expect("read"),
        source
    );
}

#[test]
fn linking_notes_produces_backlinks_in_both_directions_of_time() {
    let (_directory, vault) = vault();
    // The link is written before its target exists, which is the normal way
    // notes get written.
    vault
        .create(&id("A.md"), "See [[Borrow checker]] for why.\n")
        .expect("create");

    let mut index = open(&vault);
    assert_eq!(index.resolve("Borrow checker", None), Resolution::Missing);
    assert!(index.missing().contains_key("Borrow checker"));

    let created = vault.create(&id("Borrow checker.md"), "").expect("create");
    index.update(&created);

    let backlinks = index.backlinks(&id("Borrow checker.md"));
    assert_eq!(backlinks.len(), 1);
    assert_eq!(backlinks[0].from, id("A.md"));
    assert_eq!(backlinks[0].context, "See Borrow checker for why.");
    assert!(index.missing().is_empty());
}

#[test]
fn renaming_a_note_repoints_every_link_that_pointed_at_it() {
    let (_directory, vault) = vault();
    vault
        .create(&id("Old name.md"), "# Old name\n")
        .expect("create");
    vault
        .create(&id("A.md"), "See [[Old name]] and [[Old name|it]].\n")
        .expect("create");
    vault
        .create(&id("B.md"), "Unrelated note with no links.\n")
        .expect("create");

    let mut index = open(&vault);
    rename(&vault, &mut index, &id("Old name.md"), &id("New name.md"));

    assert_eq!(
        vault.read(&id("A.md")).expect("read").body,
        "See [[New name]] and [[New name|it]].\n"
    );
    // A note with nothing to change is not rewritten at all.
    assert_eq!(
        vault.read(&id("B.md")).expect("read").body,
        "Unrelated note with no links.\n"
    );
    assert_eq!(
        index.resolve("New name", None),
        Resolution::Note(id("New name.md"))
    );
    assert_eq!(index.backlinks(&id("New name.md")).len(), 2);
}

#[test]
fn renaming_into_a_folder_keeps_the_links_working() {
    let (_directory, vault) = vault();
    vault.create(&id("Standup.md"), "").expect("create");
    vault
        .create(&id("A.md"), "See [[Standup]].\n")
        .expect("create");

    let mut index = open(&vault);
    rename(
        &vault,
        &mut index,
        &id("Standup.md"),
        &id("Meetings/Standup.md"),
    );

    // The title did not change, so the link text did not need to.
    assert_eq!(
        vault.read(&id("A.md")).expect("read").body,
        "See [[Standup]].\n"
    );
    assert_eq!(
        index.resolve("Standup", None),
        Resolution::Note(id("Meetings/Standup.md"))
    );
}

#[test]
fn deleting_a_note_leaves_its_inbound_links_dangling_not_broken() {
    let (_directory, vault) = vault();
    vault.create(&id("A.md"), "See [[B]].\n").expect("create");
    vault.create(&id("B.md"), "").expect("create");

    let mut index = open(&vault);
    vault.delete(&id("B.md")).expect("delete");
    index.remove(&id("B.md"));

    // The link stays in the text — deleting a note must not edit other notes.
    assert_eq!(vault.read(&id("A.md")).expect("read").body, "See [[B]].\n");
    assert_eq!(index.resolve("B", None), Resolution::Missing);
    assert_eq!(index.missing().get("B"), Some(&vec![id("A.md")]));
}

#[test]
fn an_incrementally_updated_index_matches_one_built_from_a_cold_scan() {
    // The property that makes incremental updates safe: however the index got
    // into its state, it must agree with what reopening the vault would give.
    let (_directory, vault) = vault();
    let mut index = Index::build(&[]);

    for (path, body) in [
        (
            "Rust.md",
            "---\ntags: [lang]\n---\nSee [[Ownership]] and #rust.\n",
        ),
        ("Ownership.md", "Moves. See [[Rust]].\n"),
        ("Notes/Scratch.md", "#rust #project/brain ![[d.png]]\n"),
    ] {
        let note = vault.create(&id(path), body).expect("create");
        index.update(&note);
    }
    vault.delete(&id("Notes/Scratch.md")).expect("delete");
    index.remove(&id("Notes/Scratch.md"));

    let extra = vault
        .create(&id("Later.md"), "See [[Rust]].\n")
        .expect("create");
    index.update(&extra);

    let cold = open(&vault);
    assert_eq!(index.len(), cold.len());
    assert_eq!(index.tags(), cold.tags());
    assert_eq!(
        index.backlinks(&id("Rust.md")),
        cold.backlinks(&id("Rust.md"))
    );
    assert_eq!(index.missing(), cold.missing());
    for id in cold.ids() {
        assert_eq!(index.tags_of(id), cold.tags_of(id), "{id}");
        assert_eq!(index.excerpt(id), cold.excerpt(id), "{id}");
    }
}

#[test]
fn editing_frontmatter_leaves_every_other_line_of_the_file_alone() {
    let (_directory, vault) = vault();
    let source = "---\npublish: true\ntags: [rust]\ncssclass: wide\n---\n\n# Title\n\nProse.\n";
    vault.create(&id("Note.md"), source).expect("create");

    let mut note = vault.read(&id("Note.md")).expect("read");
    note.frontmatter
        .as_mut()
        .expect("frontmatter")
        .tags
        .push("learning".into());
    vault.write(&note).expect("write");

    assert_eq!(
        fs::read_to_string(vault.path_of(&id("Note.md"))).expect("read"),
        "---\npublish: true\ntags: [rust, learning]\ncssclass: wide\n---\n\n# Title\n\nProse.\n"
    );
}

#[test]
fn an_attachment_dropped_twice_is_stored_once_and_stays_referenced() {
    let (directory, vault) = vault();
    let dropped = directory.path().join("diagram.png");
    fs::write(&dropped, b"png bytes").expect("write");

    let first = vault.add_attachment(&dropped).expect("attach");
    let second = vault.add_attachment(&dropped).expect("attach");
    assert_eq!(first, second);

    vault
        .create(&id("Note.md"), &format!("![[{first}]]\n"))
        .expect("create");
    let index = open(&vault);
    assert!(index.referenced_attachments().contains(&first));

    // The attachment is not a note, however it is reached.
    assert_eq!(index.len(), 1);
}

#[test]
fn a_vault_with_notes_sharing_a_title_reports_the_ambiguity() {
    let (_directory, vault) = vault();
    vault.create(&id("Work/Standup.md"), "").expect("create");
    vault.create(&id("Home/Standup.md"), "").expect("create");
    vault
        .create(&id("A.md"), "See [[Standup]].\n")
        .expect("create");

    let index = open(&vault);
    match index.resolve("Standup", None) {
        Resolution::Ambiguous(candidates) => assert_eq!(candidates.len(), 2),
        other => panic!("expected ambiguity, got {other:?}"),
    }
    // Nothing was guessed, so nothing was backlinked either.
    assert!(index.backlinks(&id("Work/Standup.md")).is_empty());
    assert!(index.backlinks(&id("Home/Standup.md")).is_empty());

    // Writing the path form settles it.
    assert_eq!(
        index.resolve("Work/Standup", None),
        Resolution::Note(id("Work/Standup.md"))
    );
}

#[test]
fn every_note_in_a_realistic_vault_round_trips_byte_for_byte() {
    // The property that makes the vault safe to keep in git.
    let (_directory, vault) = vault();
    let sources = [
        ("Plain.md", "# Title\n\nProse.\n"),
        ("Frontmatter.md", "---\ntags: [a]\n---\nbody\n"),
        ("Block list.md", "---\ntags:\n  - a\n  - b\n---\nbody\n"),
        ("Odd spacing.md", "---\ntags:   [ a ,b ]\n---\n"),
        (
            "Unknown keys.md",
            "---\npublish: true\nbanner: \"x.png\"\n---\n",
        ),
        ("No newline at end.md", "no trailing newline"),
        ("Empty.md", ""),
        ("Unicode.md", "🎉 Héllo **wörld** [[Café]] #café\n"),
    ];
    for (path, source) in sources {
        vault.create(&id(path), source).expect("create");
    }

    for (path, source) in sources {
        let note = vault.read(&id(path)).expect("read");
        vault.write(&note).expect("write");
        assert_eq!(
            fs::read_to_string(vault.path_of(&id(path))).expect("read"),
            source,
            "{path} changed on a no-op save"
        );
    }
}

#[test]
fn a_note_referring_to_itself_by_title_is_not_ambiguous_with_another() {
    let (_directory, vault) = vault();
    vault
        .create(&id("Work/Notes.md"), "See [[Notes]].\n")
        .expect("create");
    vault.create(&id("Home/Notes.md"), "").expect("create");

    let index = open(&vault);
    assert_eq!(
        index.resolve("Notes", Some(&id("Work/Notes.md"))),
        Resolution::Note(id("Work/Notes.md"))
    );
}
