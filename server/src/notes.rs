//! The vault, as real Markdown files on the server's filesystem.
//!
//! Not a database. The point of syncing a vault at all is that the notes stay
//! ordinary files in a folder — so the copy on the NAS is one too, readable by
//! `cat`, backed up by whatever backs up that directory, and `git init`-able
//! for history. A client that stops existing leaves a vault behind.
//!
//! # Stale writes are refused, not merged
//!
//! Every write carries the hash the client last saw. If the file has moved on
//! since, the write is refused with the current hash rather than applied, and
//! the client turns that into a conflict — which is a note beside the
//! original, not a dialog. The server never merges and never picks a winner:
//! it only ever says "that is not what is here now".

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use brain_core::sync::Hash;
use serde::{Deserialize, Serialize};

/// One note as the server sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listed {
    pub id: String,
    pub hash: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListResponse {
    pub notes: Vec<Listed>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GetRequest {
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fetched {
    pub id: String,
    pub text: String,
    pub hash: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GetResponse {
    pub notes: Vec<Fetched>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PutRequest {
    pub id: String,
    pub text: String,
    /// The hash the client last saw, or `None` for "this should be new".
    #[serde(default)]
    pub base: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DeleteRequest {
    pub id: String,
    #[serde(default)]
    pub base: Option<u64>,
}

/// What the server holds now, when it refused a write.
#[derive(Debug, Serialize, Deserialize)]
pub struct Stale {
    /// Absent when the note has been deleted on the server.
    pub current: Option<u64>,
}

/// Why an id was not acceptable.
///
/// One variant, because there is exactly one recovery — do not touch the
/// filesystem — and a client that sent a bad id has a bug rather than a
/// choice.
#[derive(Debug, PartialEq, Eq)]
pub struct BadId;

/// Turn a vault-relative id into a path inside `root`, or refuse.
///
/// **This is the only thing standing between a note id off the network and the
/// server's filesystem**, so it allows rather than forbids: every segment must
/// be an ordinary name, the whole thing must end in `.md`, and anything else
/// is refused. Checking for `..` and rejecting it is the version of this that
/// misses `a/../../b`, absolute paths, and a symlink placed a week earlier.
pub fn path_of(root: &Path, id: &str) -> Result<PathBuf, BadId> {
    if id.is_empty() || !id.ends_with(".md") || id.len() > 1024 {
        return Err(BadId);
    }
    let mut path = root.to_path_buf();
    for segment in id.split('/') {
        // No empty segments, so `a//b` and a leading or trailing slash are out;
        // no `.` or `..`; nothing with a separator the host understands but
        // this loop does not; and no NUL, which some filesystems truncate at.
        if segment.is_empty()
            || segment == "."
            || segment == ".."
            || segment.starts_with('.')
            || segment.contains('\\')
            || segment.contains('\0')
        {
            return Err(BadId);
        }
        path.push(segment);
    }
    // A last check against anything the loop did not think of: whatever came
    // out must still be under the root, by components rather than by string
    // prefix.
    if !path.starts_with(root) || path.components().count() <= root.components().count() {
        return Err(BadId);
    }
    Ok(path)
}

/// The vault on disk.
pub struct Vault {
    root: PathBuf,
}

/// What a write did.
#[derive(Debug, PartialEq, Eq)]
pub enum Wrote {
    /// Applied. The note's hash now.
    Done(u64),
    /// Refused: the server has moved on since the client last looked.
    Stale(Option<u64>),
    Rejected(BadId),
    Failed(String),
}

impl Vault {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Every note, with the hash of its bytes. This is the remote snapshot the
    /// client's planner compares against.
    pub fn list(&self) -> Vec<Listed> {
        let mut notes = BTreeMap::new();
        walk(&self.root, &self.root, &mut notes);
        notes
            .into_iter()
            .map(|(id, hash)| Listed { id, hash })
            .collect()
    }

    pub fn read(&self, id: &str) -> Option<Fetched> {
        let path = path_of(&self.root, id).ok()?;
        let text = std::fs::read_to_string(path).ok()?;
        let hash = Hash::of(&text).0;
        Ok::<_, ()>(Fetched {
            id: id.to_string(),
            text,
            hash,
        })
        .ok()
    }

    fn hash_of(&self, path: &Path) -> Option<u64> {
        std::fs::read_to_string(path)
            .ok()
            .map(|text| Hash::of(&text).0)
    }

    /// Write a note, unless the server has moved on since `base`.
    pub fn write(&self, id: &str, text: &str, base: Option<u64>) -> Wrote {
        let path = match path_of(&self.root, id) {
            Ok(path) => path,
            Err(bad) => return Wrote::Rejected(bad),
        };
        let current = self.hash_of(&path);
        if current != base {
            return Wrote::Stale(current);
        }
        // Already exactly this. Not an error, and not a write either.
        let hash = Hash::of(text).0;
        if current == Some(hash) {
            return Wrote::Done(hash);
        }
        match write_atomically(&path, text) {
            Ok(()) => Wrote::Done(hash),
            Err(error) => Wrote::Failed(error.to_string()),
        }
    }

    /// Remove a note, unless the server has moved on since `base`.
    pub fn delete(&self, id: &str, base: Option<u64>) -> Wrote {
        let path = match path_of(&self.root, id) {
            Ok(path) => path,
            Err(bad) => return Wrote::Rejected(bad),
        };
        let current = self.hash_of(&path);
        if current.is_none() {
            // Already gone. The client wanted it gone, and it is.
            return Wrote::Done(0);
        }
        if current != base {
            return Wrote::Stale(current);
        }
        match std::fs::remove_file(&path) {
            Ok(()) => {
                // A folder that held only this note is now an empty directory
                // the client never asked for. Removed only if empty, and only
                // up to the root, which is the same promise Brain makes.
                prune(&self.root, path.parent());
                Wrote::Done(0)
            }
            Err(error) => Wrote::Failed(error.to_string()),
        }
    }
}

fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("md.tmp");
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, path)
}

/// Remove empty directories from `from` up to but not including `root`.
fn prune(root: &Path, from: Option<&Path>) {
    let mut current = from;
    while let Some(directory) = current {
        if directory == root || !directory.starts_with(root) {
            return;
        }
        if std::fs::remove_dir(directory).is_err() {
            return; // not empty, which is the normal case
        }
        current = directory.parent();
    }
}

fn walk(root: &Path, directory: &Path, into: &mut BTreeMap<String, u64>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        // Dotfiles are somebody else's: `.git`, `.brain`, editor swap files.
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            walk(root, &path, into);
        } else if path.extension().is_some_and(|extension| extension == "md") {
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            let Some(id) = relative.to_str() else {
                continue;
            };
            if let Ok(text) = std::fs::read_to_string(&path) {
                into.insert(id.replace('\\', "/"), Hash::of(&text).0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> (tempfile::TempDir, Vault) {
        let directory = tempfile::tempdir().expect("temp dir");
        let vault = Vault::new(directory.path().to_path_buf());
        (directory, vault)
    }

    #[test]
    fn a_note_written_can_be_listed_and_read_back() {
        let (_dir, vault) = vault();

        assert_eq!(
            vault.write("A.md", "text", None),
            Wrote::Done(Hash::of("text").0)
        );

        assert_eq!(
            vault.list(),
            vec![Listed {
                id: "A.md".into(),
                hash: Hash::of("text").0
            }]
        );
        assert_eq!(vault.read("A.md").expect("read").text, "text");
    }

    #[test]
    fn a_note_in_a_folder_keeps_its_slashes() {
        let (_dir, vault) = vault();
        vault.write("Meetings/Standup.md", "text", None);

        assert_eq!(vault.list()[0].id, "Meetings/Standup.md");
    }

    #[test]
    fn a_write_over_something_that_moved_on_is_refused_with_what_is_there() {
        let (_dir, vault) = vault();
        vault.write("A.md", "first", None);
        let theirs = Hash::of("first").0;

        // A client that never saw "first" thinks the note is new.
        assert_eq!(
            vault.write("A.md", "mine", None),
            Wrote::Stale(Some(theirs))
        );
        // And one that saw an older version is refused the same way.
        assert_eq!(
            vault.write("A.md", "mine", Some(Hash::of("older").0)),
            Wrote::Stale(Some(theirs))
        );
        // The file is untouched: the server never picks a winner.
        assert_eq!(vault.read("A.md").expect("read").text, "first");

        // Knowing what is there is what makes the write land.
        assert!(matches!(
            vault.write("A.md", "mine", Some(theirs)),
            Wrote::Done(_)
        ));
    }

    #[test]
    fn deleting_something_that_moved_on_is_refused() {
        let (_dir, vault) = vault();
        vault.write("A.md", "first", None);

        assert_eq!(
            vault.delete("A.md", Some(Hash::of("older").0)),
            Wrote::Stale(Some(Hash::of("first").0))
        );
        assert!(vault.read("A.md").is_some());
    }

    #[test]
    fn deleting_something_already_gone_is_not_an_error() {
        let (_dir, vault) = vault();

        // The client wanted it gone and it is gone. Reporting a failure would
        // make a retry loop out of a job that is finished.
        assert_eq!(vault.delete("A.md", None), Wrote::Done(0));
    }

    #[test]
    fn deleting_the_last_note_in_a_folder_takes_the_folder_with_it() {
        let (dir, vault) = vault();
        vault.write("Meetings/Standup.md", "text", None);

        vault.delete("Meetings/Standup.md", Some(Hash::of("text").0));

        assert!(!dir.path().join("Meetings").exists());
        assert!(dir.path().exists(), "the vault root was pruned");
    }

    #[test]
    fn writing_exactly_what_is_there_is_not_a_write() {
        let (_dir, vault) = vault();
        let hash = Hash::of("text").0;
        vault.write("A.md", "text", None);

        assert_eq!(vault.write("A.md", "text", Some(hash)), Wrote::Done(hash));
    }

    #[test]
    fn dotfiles_are_not_notes() {
        let (dir, vault) = vault();
        std::fs::create_dir_all(dir.path().join(".git")).expect("dir");
        std::fs::write(dir.path().join(".git/config.md"), "not a note").expect("write");
        std::fs::write(dir.path().join(".hidden.md"), "not a note").expect("write");
        vault.write("A.md", "a note", None);

        assert_eq!(vault.list().len(), 1);
    }

    // ---- the only thing between the network and the filesystem ----

    #[test]
    fn an_id_that_climbs_out_of_the_vault_is_refused() {
        let root = Path::new("/vault");

        for id in [
            "../escape.md",
            "a/../../escape.md",
            "/etc/passwd.md",
            "a/./b.md",
            "..\\escape.md",
            "",
            "no-extension",
            "a//b.md",
            "a/.hidden.md",
            ".brain/state.md",
            "a/b.md\0.md",
        ] {
            assert_eq!(path_of(root, id), Err(BadId), "{id:?} was allowed through");
        }
    }

    #[test]
    fn an_ordinary_id_is_allowed() {
        let root = Path::new("/vault");

        assert_eq!(
            path_of(root, "Meetings/2026-07-30 standup.md"),
            Ok(PathBuf::from("/vault/Meetings/2026-07-30 standup.md"))
        );
        assert_eq!(path_of(root, "A.md"), Ok(PathBuf::from("/vault/A.md")));
    }

    #[test]
    fn a_bad_id_never_reaches_the_filesystem() {
        let (_dir, vault) = vault();

        assert_eq!(
            vault.write("../escape.md", "text", None),
            Wrote::Rejected(BadId)
        );
        assert_eq!(vault.delete("../escape.md", None), Wrote::Rejected(BadId));
        assert!(vault.read("../escape.md").is_none());
    }
}
