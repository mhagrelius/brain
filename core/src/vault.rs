//! The folder of notes: scanning it, and reading and writing files in it.
//!
//! This is the external seam. Every `io::Error` is converted here into a
//! [`VaultError`] the rest of the app can reason about; above this module
//! nothing catches an error, it inspects a result.
//!
//! # Durability
//!
//! A save writes to a temporary file in the same directory, flushes it, fsyncs
//! it, and renames it over the target. A crash mid-save therefore leaves either
//! the old note or the new one, never a half-written one. The rename is atomic
//! only within a filesystem, which is why the temporary file is a sibling of
//! its target rather than in `/tmp`.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::note::{Note, NoteId};

/// Where notes are found and what is ignored.
const EXTENSION: &str = "md";
/// Brain's own cache directory inside the vault.
pub const CACHE_DIR: &str = ".brain";
/// Where dropped files are copied.
pub const ATTACHMENTS_DIR: &str = "attachments";

#[derive(Debug)]
pub enum VaultError {
    /// The path is outside the vault, or names something that is not a note.
    NotInVault(PathBuf),
    /// A note already exists where one was about to be created.
    Exists(NoteId),
    /// A folder already exists where one was about to be created or moved.
    FolderExists(String),
    /// A folder was asked to move inside itself, which `fs::rename` would
    /// either refuse or, worse, do.
    IntoItself(String),
    /// A folder still holds notes. Brain deletes one note at a time, never a
    /// subtree by accident.
    FolderNotEmpty(String),
    /// The file is not valid UTF-8. Brain does not guess encodings.
    NotText(NoteId),
    Io {
        path: PathBuf,
        source: io::Error,
    },
}

impl std::fmt::Display for VaultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInVault(path) => write!(f, "{} is not inside the vault", path.display()),
            Self::Exists(id) => write!(f, "{id} already exists"),
            Self::FolderExists(path) => write!(f, "the folder {path} already exists"),
            Self::IntoItself(path) => write!(f, "{path} cannot be moved inside itself"),
            Self::FolderNotEmpty(path) => write!(f, "{path} still holds notes"),
            Self::NotText(id) => write!(f, "{id} is not UTF-8 text"),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl std::error::Error for VaultError {}

/// A folder of Markdown files.
#[derive(Debug, Clone)]
pub struct Vault {
    root: PathBuf,
}

impl Vault {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path_of(&self, id: &NoteId) -> PathBuf {
        self.root.join(id.to_path())
    }

    /// The id of a path inside the vault, or `None` if it is outside it, is not
    /// a `.md` file, or lives somewhere Brain does not look.
    pub fn id_of(&self, path: &Path) -> Option<NoteId> {
        let relative = path.strip_prefix(&self.root).ok()?;
        if relative.extension().and_then(|e| e.to_str()) != Some(EXTENSION) {
            return None;
        }
        if relative.components().any(|component| {
            let name = component.as_os_str().to_string_lossy();
            name.starts_with('.')
        }) {
            return None;
        }
        Some(NoteId::from_relative(relative))
    }

    /// Every note in the vault, in no particular order.
    ///
    /// Unreadable files are skipped rather than failing the scan: one file with
    /// the wrong permissions must not stop the app opening. They come back in
    /// the second half of the tuple so the UI can say so once, rather than
    /// silently showing an incomplete vault.
    pub fn scan(&self) -> (Vec<Note>, Vec<VaultError>) {
        let mut notes = Vec::new();
        let mut problems = Vec::new();
        self.walk(&self.root, &mut notes, &mut problems);
        (notes, problems)
    }

    fn walk(&self, dir: &Path, notes: &mut Vec<Note>, problems: &mut Vec<VaultError>) {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(source) => {
                problems.push(VaultError::Io {
                    path: dir.to_path_buf(),
                    source,
                });
                return;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            // Dotfiles, `.brain/`, and `.git/` are not notes. Skipping them by
            // name rather than by a list keeps a vault that is also a git
            // repository from scanning its own object store.
            if name.starts_with('.') {
                continue;
            }
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => self.walk(&path, notes, problems),
                Ok(_) => {
                    let Some(id) = self.id_of(&path) else {
                        continue; // attachments and anything else that is not a note
                    };
                    match self.read(&id) {
                        Ok(note) => notes.push(note),
                        Err(problem) => problems.push(problem),
                    }
                }
                Err(source) => problems.push(VaultError::Io { path, source }),
            }
        }
    }

    pub fn read(&self, id: &NoteId) -> Result<Note, VaultError> {
        let path = self.path_of(id);
        let bytes = fs::read(&path).map_err(|source| VaultError::Io {
            path: path.clone(),
            source,
        })?;
        let text = String::from_utf8(bytes).map_err(|_| VaultError::NotText(id.clone()))?;
        Ok(Note::from_text(id.clone(), &text))
    }

    /// Write a note, atomically. Creates the containing folder if needed.
    pub fn write(&self, note: &Note) -> Result<(), VaultError> {
        self.write_bytes(&self.path_of(&note.id), note.to_text().as_bytes())
    }

    fn write_bytes(&self, path: &Path, bytes: &[u8]) -> Result<(), VaultError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| VaultError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        // A sibling of the target, so the rename stays within one filesystem
        // and is therefore atomic.
        let temporary = path.with_extension("md.tmp");
        let io = |source| VaultError::Io {
            path: temporary.clone(),
            source,
        };

        let mut file = fs::File::create(&temporary).map_err(io)?;
        file.write_all(bytes).map_err(io)?;
        file.flush().map_err(io)?;
        file.sync_all().map_err(io)?;
        drop(file);

        fs::rename(&temporary, path).map_err(|source| VaultError::Io {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Create a note, refusing to overwrite one that is already there.
    pub fn create(&self, id: &NoteId, contents: &str) -> Result<Note, VaultError> {
        if self.path_of(id).exists() {
            return Err(VaultError::Exists(id.clone()));
        }
        let note = Note::from_text(id.clone(), contents);
        self.write(&note)?;
        Ok(note)
    }

    /// Move a note. Refuses to clobber an existing one.
    pub fn rename(&self, from: &NoteId, to: &NoteId) -> Result<(), VaultError> {
        let target = self.path_of(to);
        if target.exists() {
            return Err(VaultError::Exists(to.clone()));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|source| VaultError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::rename(self.path_of(from), &target).map_err(|source| VaultError::Io {
            path: target,
            source,
        })
    }

    pub fn delete(&self, id: &NoteId) -> Result<(), VaultError> {
        let path = self.path_of(id);
        fs::remove_file(&path).map_err(|source| VaultError::Io { path, source })
    }

    /// When a note was last written and when it was created, in seconds since
    /// the epoch.
    ///
    /// Creation time is not something every filesystem records, so it falls
    /// back to the modification time rather than to zero — a note that exists
    /// was created at some point, and sorting it to the bottom for ever
    /// because ext3 does not keep birth times would be worse than approximate.
    pub fn times(&self, id: &NoteId) -> (u64, u64) {
        let Ok(metadata) = fs::metadata(self.path_of(id)) else {
            return (0, 0);
        };
        let seconds = |time: io::Result<std::time::SystemTime>| {
            time.ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|since| since.as_secs())
        };
        let modified = seconds(metadata.modified()).unwrap_or(0);
        (modified, seconds(metadata.created()).unwrap_or(modified))
    }

    // ---- folders ----

    /// Every folder in the vault, vault-relative and `/` separated.
    ///
    /// Listed from the filesystem rather than derived from the notes, because a
    /// folder with nothing in it yet is exactly the folder someone just made
    /// and is about to look for. `attachments/` is not one of them: it holds
    /// files Brain put there, not notes anyone files.
    pub fn folders(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.walk_folders(&self.root, "", &mut out);
        out.sort();
        out
    }

    fn walk_folders(&self, dir: &Path, prefix: &str, out: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || (prefix.is_empty() && name == ATTACHMENTS_DIR) {
                continue;
            }
            let path = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            self.walk_folders(&entry.path(), &path, out);
            out.push(path);
        }
    }

    /// The absolute path of a vault-relative folder. The empty string is the
    /// vault root.
    pub fn folder_path(&self, folder: &str) -> PathBuf {
        if folder.is_empty() {
            self.root.clone()
        } else {
            self.root.join(folder)
        }
    }

    pub fn create_folder(&self, folder: &str) -> Result<(), VaultError> {
        let path = self.folder_path(folder);
        if path.exists() {
            return Err(VaultError::FolderExists(folder.to_string()));
        }
        fs::create_dir_all(&path).map_err(|source| VaultError::Io { path, source })
    }

    /// Move or rename a folder, with everything in it.
    ///
    /// One `fs::rename` rather than a note-by-note walk: the directory moves
    /// atomically, so an interrupted move cannot leave half a folder in each
    /// place. Every note under it changes id, which the caller handles by
    /// rescanning — the alternative is patching a subtree of the index and
    /// getting it subtly wrong.
    pub fn move_folder(&self, from: &str, to: &str) -> Result<(), VaultError> {
        if from.is_empty() {
            return Err(VaultError::NotInVault(self.root.clone()));
        }
        if crate::tree::is_within(from, to) {
            return Err(VaultError::IntoItself(from.to_string()));
        }
        let target = self.folder_path(to);
        if target.exists() {
            return Err(VaultError::FolderExists(to.to_string()));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|source| VaultError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::rename(self.folder_path(from), &target).map_err(|source| VaultError::Io {
            path: target,
            source,
        })
    }

    /// Remove a folder that holds no notes.
    ///
    /// Refused while anything is still in it. Deleting a subtree is one click
    /// away from deleting a year of notes, and the file manager is a better
    /// place to mean it.
    pub fn delete_folder(&self, folder: &str) -> Result<(), VaultError> {
        if folder.is_empty() {
            return Err(VaultError::NotInVault(self.root.clone()));
        }
        let path = self.folder_path(folder);
        let empty = fs::read_dir(&path)
            .map(|entries| entries.flatten().all(|entry| entry.path().is_dir()))
            .unwrap_or(false);
        let has_notes = self
            .scan()
            .0
            .iter()
            .any(|note| note.id.as_str().starts_with(&format!("{folder}/")));
        if has_notes || !empty {
            return Err(VaultError::FolderNotEmpty(folder.to_string()));
        }
        fs::remove_dir_all(&path).map_err(|source| VaultError::Io { path, source })
    }

    /// Copy a dropped file into `attachments/`, returning the name to embed.
    ///
    /// A name already in use by identical content is reused rather than
    /// duplicated; one in use by *different* content gets a numeric suffix, so
    /// dropping two files called `screenshot.png` keeps both.
    pub fn add_attachment(&self, source: &Path) -> Result<String, VaultError> {
        let bytes = fs::read(source).map_err(|error| VaultError::Io {
            path: source.to_path_buf(),
            source: error,
        })?;
        let name = source
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .ok_or_else(|| VaultError::NotInVault(source.to_path_buf()))?;

        let directory = self.root.join(ATTACHMENTS_DIR);
        let (stem, extension) = match name.rsplit_once('.') {
            Some((stem, extension)) => (stem.to_string(), format!(".{extension}")),
            None => (name.clone(), String::new()),
        };

        for attempt in 0.. {
            let candidate = if attempt == 0 {
                name.clone()
            } else {
                format!("{stem}-{attempt}{extension}")
            };
            let path = directory.join(&candidate);
            match fs::read(&path) {
                // Same name, same bytes: it is already here.
                Ok(existing) if existing == bytes => return Ok(candidate),
                Ok(_) => continue,
                Err(_) => {
                    self.write_bytes(&path, &bytes)?;
                    return Ok(candidate);
                }
            }
        }
        unreachable!("the loop returns on the first free name")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn vault() -> (tempfile::TempDir, Vault) {
        let directory = tempfile::tempdir().expect("temp dir");
        let vault = Vault::new(directory.path());
        (directory, vault)
    }

    fn id(path: &str) -> NoteId {
        NoteId::from_relative(path)
    }

    fn titles(notes: &[Note]) -> BTreeSet<String> {
        notes.iter().map(|n| n.id.as_str().to_string()).collect()
    }

    #[test]
    fn a_written_note_reads_back_identically() {
        let (_directory, vault) = vault();
        let source = "---\ntags: [rust]\n---\n\n# Title\n\nProse.\n";
        vault.create(&id("Note.md"), source).expect("create");

        let read = vault.read(&id("Note.md")).expect("read");
        assert_eq!(read.to_text(), source);
    }

    #[test]
    fn scanning_finds_notes_in_folders_and_ignores_everything_else() {
        let (directory, vault) = vault();
        vault.create(&id("Top.md"), "top").expect("create");
        vault
            .create(&id("Meetings/Standup.md"), "standup")
            .expect("create");
        // Not notes: an attachment, a dotfile, and Brain's own cache.
        fs::create_dir_all(directory.path().join(ATTACHMENTS_DIR)).expect("dir");
        fs::write(directory.path().join("attachments/d.png"), b"png").expect("write");
        fs::create_dir_all(directory.path().join(CACHE_DIR)).expect("dir");
        fs::write(directory.path().join(".brain/index.json"), b"{}").expect("write");
        fs::write(directory.path().join(".hidden.md"), b"hidden").expect("write");

        let (notes, problems) = vault.scan();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            titles(&notes),
            ["Meetings/Standup.md".to_string(), "Top.md".to_string()].into()
        );
    }

    #[test]
    fn a_vault_that_is_a_git_repository_does_not_scan_its_own_objects() {
        let (directory, vault) = vault();
        vault.create(&id("Note.md"), "note").expect("create");
        let objects = directory.path().join(".git/objects");
        fs::create_dir_all(&objects).expect("dir");
        fs::write(objects.join("deadbeef.md"), b"not a note").expect("write");

        let (notes, _) = vault.scan();
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn scanning_an_empty_folder_is_not_an_error() {
        let (_directory, vault) = vault();
        let (notes, problems) = vault.scan();
        assert!(notes.is_empty());
        assert!(problems.is_empty());
    }

    #[test]
    fn creating_over_an_existing_note_is_refused() {
        let (_directory, vault) = vault();
        vault.create(&id("Note.md"), "first").expect("create");
        assert!(matches!(
            vault.create(&id("Note.md"), "second"),
            Err(VaultError::Exists(_))
        ));
        // And the original is untouched.
        assert_eq!(vault.read(&id("Note.md")).expect("read").body, "first");
    }

    #[test]
    fn renaming_moves_the_file_and_can_create_a_folder() {
        let (_directory, vault) = vault();
        vault.create(&id("Note.md"), "body").expect("create");
        vault
            .rename(&id("Note.md"), &id("Archive/Note.md"))
            .expect("rename");

        assert!(vault.read(&id("Note.md")).is_err());
        assert_eq!(
            vault.read(&id("Archive/Note.md")).expect("read").body,
            "body"
        );
    }

    #[test]
    fn renaming_onto_an_existing_note_is_refused() {
        let (_directory, vault) = vault();
        vault.create(&id("A.md"), "a").expect("create");
        vault.create(&id("B.md"), "b").expect("create");
        assert!(matches!(
            vault.rename(&id("A.md"), &id("B.md")),
            Err(VaultError::Exists(_))
        ));
        assert_eq!(vault.read(&id("B.md")).expect("read").body, "b");
    }

    #[test]
    fn a_save_leaves_no_temporary_file_behind() {
        let (directory, vault) = vault();
        vault.create(&id("Note.md"), "body").expect("create");
        let leftovers: Vec<_> = fs::read_dir(directory.path())
            .expect("read dir")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn a_file_that_is_not_utf8_is_reported_not_guessed_at() {
        let (directory, vault) = vault();
        fs::write(directory.path().join("Bad.md"), [0xff, 0xfe, 0x00]).expect("write");
        assert!(matches!(
            vault.read(&id("Bad.md")),
            Err(VaultError::NotText(_))
        ));
        // The scan reports it and carries on with the rest of the vault.
        vault.create(&id("Good.md"), "good").expect("create");
        let (notes, problems) = vault.scan();
        assert_eq!(notes.len(), 1);
        assert_eq!(problems.len(), 1);
    }

    #[test]
    fn ids_are_only_issued_for_notes_inside_the_vault() {
        let (directory, vault) = vault();
        assert_eq!(
            vault.id_of(&directory.path().join("Note.md")),
            Some(id("Note.md"))
        );
        assert_eq!(
            vault.id_of(&directory.path().join("a/b.md")),
            Some(id("a/b.md"))
        );
        assert_eq!(vault.id_of(&directory.path().join("d.png")), None);
        assert_eq!(vault.id_of(&directory.path().join(".brain/x.md")), None);
        assert_eq!(vault.id_of(Path::new("/elsewhere/Note.md")), None);
    }

    #[test]
    fn folders_are_listed_from_disk_including_the_empty_ones() {
        let (directory, vault) = vault();
        vault
            .create(&id("Work/Deep/Note.md"), "note")
            .expect("create");
        vault.create_folder("Archive").expect("folder");
        // Not folders anyone files notes in.
        fs::create_dir_all(directory.path().join(ATTACHMENTS_DIR)).expect("dir");
        fs::create_dir_all(directory.path().join(CACHE_DIR)).expect("dir");

        assert_eq!(vault.folders(), ["Archive", "Work", "Work/Deep"]);
    }

    #[test]
    fn a_folder_moves_with_everything_in_it() {
        let (_directory, vault) = vault();
        vault.create(&id("Work/Note.md"), "body").expect("create");
        vault.move_folder("Work", "Archive/Work").expect("move");

        assert_eq!(
            vault.read(&id("Archive/Work/Note.md")).expect("read").body,
            "body"
        );
        assert!(vault.read(&id("Work/Note.md")).is_err());
    }

    #[test]
    fn a_folder_cannot_be_moved_inside_itself() {
        let (_directory, vault) = vault();
        vault.create(&id("Work/Note.md"), "body").expect("create");
        assert!(matches!(
            vault.move_folder("Work", "Work/Nested"),
            Err(VaultError::IntoItself(_))
        ));
        assert!(vault.read(&id("Work/Note.md")).is_ok());
    }

    #[test]
    fn moving_onto_an_existing_folder_is_refused() {
        let (_directory, vault) = vault();
        vault.create(&id("A/Note.md"), "a").expect("create");
        vault.create(&id("B/Note.md"), "b").expect("create");
        assert!(matches!(
            vault.move_folder("A", "B"),
            Err(VaultError::FolderExists(_))
        ));
        assert_eq!(vault.read(&id("B/Note.md")).expect("read").body, "b");
    }

    #[test]
    fn a_folder_with_notes_in_it_is_not_deleted() {
        let (_directory, vault) = vault();
        vault.create(&id("Work/Note.md"), "body").expect("create");
        assert!(matches!(
            vault.delete_folder("Work"),
            Err(VaultError::FolderNotEmpty(_))
        ));
        assert!(vault.read(&id("Work/Note.md")).is_ok());

        vault.delete(&id("Work/Note.md")).expect("delete");
        vault.delete_folder("Work").expect("delete folder");
        assert!(vault.folders().is_empty());
    }

    #[test]
    fn a_notes_times_are_read_and_an_unknown_one_is_zero() {
        let (_directory, vault) = vault();
        vault.create(&id("Note.md"), "body").expect("create");
        let (modified, created) = vault.times(&id("Note.md"));
        assert!(modified > 0, "a note just written has a modification time");
        assert!(created > 0, "creation falls back to modification");

        assert_eq!(vault.times(&id("Gone.md")), (0, 0));
    }

    #[test]
    fn an_attachment_is_copied_in_and_named_once() {
        let (directory, vault) = vault();
        let source = directory.path().join("outside.png");
        fs::write(&source, b"image bytes").expect("write");

        let name = vault.add_attachment(&source).expect("attach");
        assert_eq!(name, "outside.png");
        assert_eq!(
            fs::read(directory.path().join("attachments/outside.png")).expect("read"),
            b"image bytes"
        );

        // The same file again is the same attachment, not a second copy.
        assert_eq!(
            vault.add_attachment(&source).expect("attach"),
            "outside.png"
        );
    }

    #[test]
    fn two_different_files_with_one_name_both_survive() {
        let (directory, vault) = vault();
        let first = directory.path().join("a/screenshot.png");
        let second = directory.path().join("b/screenshot.png");
        fs::create_dir_all(first.parent().expect("parent")).expect("dir");
        fs::create_dir_all(second.parent().expect("parent")).expect("dir");
        fs::write(&first, b"one").expect("write");
        fs::write(&second, b"two").expect("write");

        assert_eq!(
            vault.add_attachment(&first).expect("attach"),
            "screenshot.png"
        );
        assert_eq!(
            vault.add_attachment(&second).expect("attach"),
            "screenshot-1.png"
        );
    }
}
