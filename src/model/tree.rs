//! The sidebar's shape: folders, and the notes inside them.
//!
//! The vault is the tree. Nothing here is stored — a folder exists because a
//! directory does, a note sits under a folder because its path says so, and the
//! order within a folder comes from a sort the user picked, never from anything
//! Brain wrote down. That is why moving a note is a file rename and nothing
//! else, and why a folder created in Files shows up here with no import step.
//!
//! Which folders are open is the one piece of view state, and it is the
//! caller's: this function takes it and returns a flat list of rows, so the
//! widget never walks a tree and the walking is tested without a display.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::note::NoteId;

/// How notes are ordered inside a folder.
///
/// Folders are always alphabetical whichever of these is chosen: a folder has
/// no useful date of its own, and a column of folders that reshuffles is a
/// column you cannot learn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    #[default]
    Name,
    Modified,
    Created,
}

impl Sort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Modified => "modified",
            Self::Created => "created",
        }
    }

    /// Parse what was stored in the config. An unknown value is the default
    /// rather than an error: a config from a future version must not stop the
    /// sidebar drawing.
    pub fn from_name(name: &str) -> Self {
        match name {
            "modified" => Self::Modified,
            "created" => Self::Created,
            _ => Self::Name,
        }
    }
}

/// A note as the sidebar needs it: what to show, and what to sort by.
///
/// The times are seconds since the epoch, and `0` means the filesystem did not
/// say — an unknown time sorts last rather than first, so a file whose
/// timestamp could not be read does not claim to be the newest thing there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub id: NoteId,
    pub excerpt: String,
    pub modified: u64,
    pub created: u64,
}

impl Listed {
    /// A note with no timestamps, for callers that only ever sort by name.
    pub fn new(id: NoteId, excerpt: String) -> Self {
        Self {
            id,
            excerpt,
            modified: 0,
            created: 0,
        }
    }
}

/// One line of the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Folder {
        /// Vault-relative, `/` separated. The folder's identity.
        path: String,
        /// The last segment, which is what the row shows.
        name: String,
        depth: usize,
        /// Notes anywhere beneath it, so a closed folder still says how much it
        /// holds.
        notes: usize,
        expanded: bool,
    },
    Note {
        id: NoteId,
        excerpt: String,
        depth: usize,
    },
}

impl Row {
    pub fn depth(&self) -> usize {
        match self {
            Self::Folder { depth, .. } | Self::Note { depth, .. } => *depth,
        }
    }
}

/// The rows to draw, top to bottom.
///
/// `folders` carries directories that hold no notes — those exist on disk and
/// have to appear, or making a folder and then looking for it is a bug report.
/// Ancestors are implied: a note at `a/b/c.md` puts both `a` and `a/b` in the
/// tree whether or not anyone listed them.
pub fn rows(
    notes: &[Listed],
    folders: &[String],
    expanded: &BTreeSet<String>,
    sort: Sort,
) -> Vec<Row> {
    let mut children: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut contents: BTreeMap<String, Vec<&Listed>> = BTreeMap::new();

    let register = |path: &str, children: &mut BTreeMap<String, BTreeSet<String>>| {
        let mut parent = String::new();
        for segment in path.split('/').filter(|segment| !segment.is_empty()) {
            let full = if parent.is_empty() {
                segment.to_string()
            } else {
                format!("{parent}/{segment}")
            };
            children
                .entry(parent.clone())
                .or_default()
                .insert(full.clone());
            children.entry(full.clone()).or_default();
            parent = full;
        }
    };

    for folder in folders {
        register(folder, &mut children);
    }
    for note in notes {
        let folder = note.id.folder().unwrap_or("");
        register(folder, &mut children);
        contents.entry(folder.to_string()).or_default().push(note);
    }
    children.entry(String::new()).or_default();

    let mut out = Vec::new();
    emit("", 0, &children, &contents, expanded, sort, &mut out);
    out
}

/// Folders first, then the notes sitting directly in this folder — the order
/// every file manager uses, and the one that keeps a folder's own notes from
/// being lost between its subfolders.
fn emit(
    folder: &str,
    depth: usize,
    children: &BTreeMap<String, BTreeSet<String>>,
    contents: &BTreeMap<String, Vec<&Listed>>,
    expanded: &BTreeSet<String>,
    sort: Sort,
    out: &mut Vec<Row>,
) {
    if let Some(subfolders) = children.get(folder) {
        for path in subfolders {
            let open = expanded.contains(path);
            out.push(Row::Folder {
                path: path.clone(),
                name: path.rsplit('/').next().unwrap_or(path).to_string(),
                depth,
                notes: count(path, children, contents),
                expanded: open,
            });
            if open {
                emit(path, depth + 1, children, contents, expanded, sort, out);
            }
        }
    }

    let Some(notes) = contents.get(folder) else {
        return;
    };
    let mut notes = notes.clone();
    sort_notes(&mut notes, sort);
    out.extend(notes.into_iter().map(|note| Row::Note {
        id: note.id.clone(),
        excerpt: note.excerpt.clone(),
        depth,
    }));
}

fn count(
    folder: &str,
    children: &BTreeMap<String, BTreeSet<String>>,
    contents: &BTreeMap<String, Vec<&Listed>>,
) -> usize {
    let here = contents.get(folder).map(Vec::len).unwrap_or(0);
    let beneath: usize = children
        .get(folder)
        .into_iter()
        .flatten()
        .map(|child| count(child, children, contents))
        .sum();
    here + beneath
}

/// Newest first for the two time sorts, because "recently" is the point of
/// asking for them. Ties fall back to the name, so the list never reshuffles
/// between two notes written in the same second.
fn sort_notes(notes: &mut [&Listed], sort: Sort) {
    match sort {
        Sort::Name => notes.sort_by(|a, b| by_name(a, b)),
        Sort::Modified => {
            notes.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| by_name(a, b)))
        }
        Sort::Created => {
            notes.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| by_name(a, b)))
        }
    }
}

fn by_name(a: &Listed, b: &Listed) -> std::cmp::Ordering {
    a.id.title()
        .to_lowercase()
        .cmp(&b.id.title().to_lowercase())
        .then_with(|| a.id.cmp(&b.id))
}

/// Whether `folder` is `into` or sits beneath it.
///
/// A folder cannot be dropped inside itself: `fs::rename` would either fail or,
/// worse, succeed on some filesystems and lose the subtree.
pub fn is_within(folder: &str, into: &str) -> bool {
    folder == into || into.starts_with(&format!("{folder}/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(path: &str, modified: u64, created: u64) -> Listed {
        Listed {
            id: NoteId::from_relative(path),
            excerpt: String::new(),
            modified,
            created,
        }
    }

    fn open(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|p| p.to_string()).collect()
    }

    /// The rows as `depth:name`, which is what a sidebar looks like.
    fn shape(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                Row::Folder { name, depth, .. } => format!("{depth}:{name}/"),
                Row::Note { id, depth, .. } => format!("{depth}:{}", id.title()),
            })
            .collect()
    }

    #[test]
    fn a_closed_folder_hides_its_notes_and_an_open_one_shows_them() {
        let notes = [
            listed("Meetings/Standup.md", 0, 0),
            listed("Inbox.md", 0, 0),
        ];

        let closed = rows(&notes, &[], &open(&[]), Sort::Name);
        assert_eq!(shape(&closed), ["0:Meetings/", "0:Inbox"]);

        let opened = rows(&notes, &[], &open(&["Meetings"]), Sort::Name);
        assert_eq!(shape(&opened), ["0:Meetings/", "1:Standup", "0:Inbox"]);
    }

    #[test]
    fn folders_come_before_the_notes_beside_them() {
        // Otherwise a folder's own notes are lost between its subfolders.
        let notes = [
            listed("Apple.md", 0, 0),
            listed("Zebra/Note.md", 0, 0),
            listed("Banana.md", 0, 0),
        ];
        assert_eq!(
            shape(&rows(&notes, &[], &open(&[]), Sort::Name)),
            ["0:Zebra/", "0:Apple", "0:Banana"]
        );
    }

    #[test]
    fn ancestors_appear_even_when_nothing_named_them() {
        let notes = [listed("a/b/c/Deep.md", 0, 0)];
        assert_eq!(
            shape(&rows(
                &notes,
                &[],
                &open(&["a", "a/b", "a/b/c"]),
                Sort::Name
            )),
            ["0:a/", "1:b/", "2:c/", "3:Deep"]
        );
    }

    #[test]
    fn an_empty_folder_on_disk_is_still_a_folder() {
        // Make a folder, then look for it. Anything else is a bug report.
        let rows = rows(&[], &["Archive".to_string()], &open(&[]), Sort::Name);
        assert_eq!(shape(&rows), ["0:Archive/"]);
    }

    #[test]
    fn a_folder_counts_every_note_beneath_it_not_only_its_own() {
        let notes = [
            listed("Work/A.md", 0, 0),
            listed("Work/Deep/B.md", 0, 0),
            listed("Work/Deep/C.md", 0, 0),
        ];
        let rows = rows(&notes, &[], &open(&[]), Sort::Name);
        match &rows[0] {
            Row::Folder { notes, .. } => assert_eq!(*notes, 3),
            other => panic!("expected a folder, got {other:?}"),
        }
    }

    #[test]
    fn sorting_by_time_puts_the_newest_first_and_folders_stay_alphabetical() {
        let notes = [
            listed("Old.md", 100, 300),
            listed("New.md", 300, 100),
            listed("Middle.md", 200, 200),
        ];
        assert_eq!(
            shape(&rows(&notes, &[], &open(&[]), Sort::Modified)),
            ["0:New", "0:Middle", "0:Old"]
        );
        assert_eq!(
            shape(&rows(&notes, &[], &open(&[]), Sort::Created)),
            ["0:Old", "0:Middle", "0:New"]
        );
    }

    #[test]
    fn notes_with_the_same_time_keep_a_stable_order() {
        let notes = [listed("B.md", 5, 5), listed("A.md", 5, 5)];
        assert_eq!(
            shape(&rows(&notes, &[], &open(&[]), Sort::Modified)),
            ["0:A", "0:B"]
        );
    }

    #[test]
    fn a_note_whose_time_is_unknown_does_not_claim_to_be_the_newest() {
        let notes = [listed("Unknown.md", 0, 0), listed("Known.md", 10, 10)];
        assert_eq!(
            shape(&rows(&notes, &[], &open(&[]), Sort::Modified)),
            ["0:Known", "0:Unknown"]
        );
    }

    #[test]
    fn names_sort_without_regard_to_case() {
        let notes = [listed("apple.md", 0, 0), listed("Banana.md", 0, 0)];
        assert_eq!(
            shape(&rows(&notes, &[], &open(&[]), Sort::Name)),
            ["0:apple", "0:Banana"]
        );
    }

    #[test]
    fn an_empty_vault_is_no_rows_and_no_panic() {
        assert!(rows(&[], &[], &open(&[]), Sort::Name).is_empty());
    }

    #[test]
    fn a_folder_is_within_itself_and_its_descendants_only() {
        assert!(is_within("Work", "Work"));
        assert!(is_within("Work", "Work/Deep"));
        assert!(!is_within("Work", "Working"));
        assert!(!is_within("Work", "Other"));
        assert!(!is_within("Work/Deep", "Work"));
    }

    #[test]
    fn a_sort_round_trips_through_the_config_and_a_stranger_is_the_default() {
        for sort in [Sort::Name, Sort::Modified, Sort::Created] {
            assert_eq!(Sort::from_name(sort.as_str()), sort);
        }
        assert_eq!(
            Sort::from_name("whatever the next version writes"),
            Sort::Name
        );
    }
}
