//! The notebook: everything Brain does to a vault, with nothing that draws.
//!
//! This is the headless half of what used to be `ui::BrainApplication` — the
//! vault, the index, the open note, the sidebar's state, and every rule about
//! what happens when one of them changes. The shell owns a `Notebook` and is
//! reduced to three jobs: hand it what the user did, ask it what to display,
//! and turn the outcome it returns into toasts and redraws.
//!
//! **The `Notebook` is the only thing that writes a file or mutates the index.**
//! That rule used to name `BrainApplication`; moving it here did not weaken it,
//! it only put it somewhere `cargo test` can reach without a display.
//!
//! # Why the methods return outcomes
//!
//! A mutation says what happened to the vault, not what to put on screen:
//! [`Renamed`] carries how many links were rewritten, not the sentence about
//! them. The wording, the toast, and which panes need rebuilding are the
//! shell's business, and a second shell on another platform will word them
//! differently. What both shells must agree on is what happened — so that is
//! what crosses the line, and it is what the tests assert about.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::bm25::Bm25;
use crate::config::Config;
use crate::index::{Index, Resolution};
use crate::markdown;
use crate::note::{Note, NoteId};
use crate::search;
use crate::semantic;
use crate::tree::{self, Listed, Row, Sort};
use crate::vault::{Vault, VaultError};

/// What a palette query is matched against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Titles, aliases and paths, fuzzily.
    #[default]
    Title,
    /// The text of every note.
    Text,
}

/// One palette row: a note, what to say about it, and which part to highlight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// The note's id, reported verbatim when the row is chosen.
    pub id: String,
    pub title: String,
    /// The folder, or the matching line in text mode.
    pub detail: String,
    /// Character range within `detail` to mark, if any.
    pub highlight: Option<(usize, usize)>,
}

/// Why an operation on the vault did not happen.
///
/// "There is no vault open" is not a [`VaultError`] — there is no path to name
/// in the message — but it is the same kind of answer to the same question, so
/// the callers that can meet both get one type rather than an `Option<Result>`.
// No `PartialEq`: `VaultError` carries an `io::Error`, which has no equality.
// Tests match on the variant, which is the question they are actually asking.
#[derive(Debug)]
pub enum Failed {
    /// Asked to change a vault when none is open. Every such call is a no-op;
    /// the shell gates its controls on this rather than reporting it.
    NoVault,
    Vault(VaultError),
}

impl From<VaultError> for Failed {
    fn from(error: VaultError) -> Self {
        Self::Vault(error)
    }
}

impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoVault => write!(f, "no vault is open"),
            Self::Vault(error) => error.fmt(f),
        }
    }
}

/// What [`Notebook::save_now`] did.
#[derive(Debug)]
pub enum Saved {
    /// Nothing was waiting to be written.
    Clean,
    Written,
    /// The note is still dirty, so the next tick tries again. A note that
    /// failed to save must not be quietly forgotten.
    Failed(VaultError),
}

/// What [`Notebook::rename_note`] did.
#[derive(Debug)]
pub enum Renamed {
    /// No open note, no vault, or the title did not change.
    Unchanged,
    Done {
        to: NoteId,
        /// Inbound links rewritten to the new title.
        links: usize,
    },
    Failed(VaultError),
}

/// What [`Notebook::move_note`] did.
#[derive(Debug)]
pub enum Moved {
    /// Already there, or there is no vault.
    Unchanged,
    Done {
        to: NoteId,
        /// Empty for the vault root.
        destination: String,
    },
    Failed(VaultError),
}

/// What [`Notebook::absorb_external_changes`] found.
///
/// The open note is the delicate part: reloading it would throw away whatever
/// is being typed.
#[derive(Debug, PartialEq)]
pub enum External {
    /// Nothing was open, or the file is byte for byte what Brain last saw.
    /// The vault around it has still been rescanned.
    Quiet,
    /// The open note was reloaded from disk, because nothing local was unsaved.
    /// This is what makes `git checkout` feel right.
    Reloaded,
    /// The open note changed on disk *and* has unsaved local edits. Both
    /// versions still exist — this is the one case that has to be asked about.
    Diverged { id: NoteId, on_disk: String },
    /// The open note is no longer in the vault. It is still open here.
    Vanished { id: NoteId },
}

/// What the banner is saying, when more than one thing could be wrong at once.
///
/// The order of the variants is the priority order, and it is deliberate.
/// [`Alert::NotSaving`] outranks the others because it is the only one where
/// work is being lost *now*: in both of the others the two versions are safely
/// on disk and in the editor, and the user is being asked which they want. A
/// divergence hidden behind a save failure would be a nuisance; a save failure
/// hidden behind a divergence would be a lost note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Alert {
    /// Writing the open note is failing, and the note is still dirty.
    NotSaving(String),
    /// The open note changed on disk and has unsaved local edits. Both
    /// versions still exist; [`Notebook::take_disk_version`] picks theirs and
    /// doing nothing keeps yours.
    Diverged(NoteId),
    /// The open note's file is gone from the vault. It is still in the editor,
    /// and [`Notebook::restore_open_note`] writes it back.
    Vanished(NoteId),
}

/// What [`Notebook::attach_files`] managed.
#[derive(Debug, Default, PartialEq)]
pub struct Attached {
    /// Names to insert as embeds, in the order they were dropped.
    pub names: Vec<String>,
    pub failures: usize,
}

/// Everything Brain knows about one vault.
///
/// Deliberately not `Clone`: two of these over one folder would be two writers
/// and two indexes disagreeing about the same notes.
#[derive(Debug, Default)]
pub struct Notebook {
    config: Config,
    config_path: PathBuf,
    vault: Option<Vault>,
    index: Index,
    /// The note in the editor, if any.
    open: Option<NoteId>,
    /// The open note as it stands, including unsaved edits.
    buffer: Option<Note>,
    /// The text of the open note as it last stood *on disk*, whether Brain put
    /// it there or read it from there. A watcher cannot tell whose write it is
    /// reporting, so this is what tells Brain's own saves apart from somebody
    /// else's edit.
    on_disk: Option<String>,
    dirty: bool,
    /// The tag filtering the note list, if any.
    filter: Option<String>,
    /// What the sidebar's search entry holds. While it is not empty the sidebar
    /// shows results rather than the tree.
    query: String,
    /// Which folders are open in the sidebar.
    expanded: BTreeSet<String>,
    sort: Sort,
    /// The folder a new note goes in: whichever was last chosen in the sidebar,
    /// falling back to the open note's.
    target_folder: Option<String>,
    /// BM25 counts over the current index, rebuilt with it. Cheap enough to
    /// rebuild and wrong enough to keep if it is not.
    lexical: Bm25,
    /// The vectors, and where they are cached. Derived data: losing the file
    /// costs one pass of embedding.
    vectors: semantic::Store,
    vectors_path: Option<PathBuf>,
    /// The last query embedded, and its vector. One entry, because the only
    /// query worth having a vector for is the one in the palette now.
    query_vector: Option<(String, Vec<f32>)>,
    /// The three conditions the banner can be reporting. Held separately
    /// rather than as one slot, because they arise and clear independently —
    /// a divergence that was pushed off the banner by a save failure has to
    /// still be there when the save starts working again.
    not_saving: Option<String>,
    diverged: Option<NoteId>,
    vanished: Option<NoteId>,
}

impl Notebook {
    // ---- configuration ----

    /// Read the config and open whatever vault it names.
    ///
    /// A configured folder that no longer exists leaves the notebook without a
    /// vault rather than with a broken one, which is the state the first-run
    /// chooser is for.
    pub fn load_config(&mut self, path: PathBuf) {
        let (config, _outcome) = Config::load(&path);
        self.config_path = path;

        if let Some(root) = config.vault.clone() {
            if root.is_dir() {
                self.vault = Some(Vault::new(root));
            }
        }
        self.sort = Sort::from_name(&config.sort);
        self.expanded = config.expanded_folders.iter().cloned().collect();
        self.config = config;
        self.load_vectors();
    }

    /// Write the config back, folding in the view state that lives on `self`.
    ///
    /// Losing this costs one trip through the folder chooser, so the error is
    /// returned for a log line rather than a dialog.
    pub fn save_config(&mut self) -> Result<(), crate::config::ConfigError> {
        self.config.sort = self.sort.as_str().to_string();
        self.config.expanded_folders = self.expanded.iter().cloned().collect();
        self.config.save(&self.config_path)
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    /// Where the embedding server is, or `None` when semantic search is off.
    ///
    /// Off is a configured empty string, not a missing key: a vault with no
    /// server reachable behaves the same as one with the feature turned off,
    /// and the difference is only whether Brain keeps trying.
    pub fn embedding_url(&self, default: &str) -> Option<String> {
        match self.config.embedding_url.clone() {
            Some(url) if url.trim().is_empty() => None,
            Some(url) => Some(url),
            None => Some(default.to_string()),
        }
    }

    // ---- the vault ----

    pub fn vault(&self) -> Option<&Vault> {
        self.vault.as_ref()
    }

    pub fn vault_root(&self) -> Option<PathBuf> {
        self.vault.as_ref().map(|vault| vault.root().to_path_buf())
    }

    pub fn has_vault(&self) -> bool {
        self.vault.is_some()
    }

    /// Point at a different vault folder.
    ///
    /// The vectors are loaded before the rescan, so the catch-up the rescan
    /// schedules starts from whatever this vault already had embedded rather
    /// than from the previous vault's.
    pub fn set_vault(&mut self, root: &Path) -> Vec<VaultError> {
        self.open = None;
        self.buffer = None;
        self.on_disk = None;
        self.dirty = false;
        self.clear_alerts();
        self.vault = Some(Vault::new(root));
        self.config.vault = Some(root.to_path_buf());
        self.config.last_note = None;

        self.load_vectors();
        self.rescan()
    }

    /// Rebuild the index from the folder. Returns the files that could not be
    /// read — one bad permission must not stop the app opening, and must not be
    /// silent either.
    pub fn rescan(&mut self) -> Vec<VaultError> {
        let Some(vault) = self.vault.clone() else {
            self.index = Index::default();
            self.lexical = Bm25::default();
            return Vec::new();
        };
        let (notes, problems) = vault.scan();
        self.index = Index::build(&notes);
        self.lexical = Bm25::build(&self.index);
        problems
    }

    /// Rescan, and put the open note back if it survived.
    pub fn reload_vault(&mut self) -> Vec<VaultError> {
        let problems = self.rescan();
        match self.open.clone().filter(|id| self.index.contains(id)) {
            Some(id) => {
                let _ = self.load_note(&id);
            }
            None => {
                self.open = None;
                self.buffer = None;
                self.on_disk = None;
            }
        }
        problems
    }

    /// What the banner should be reporting, if anything.
    ///
    /// Highest priority first — see [`Alert`] for why that order.
    pub fn alert(&self) -> Option<Alert> {
        if let Some(message) = &self.not_saving {
            return Some(Alert::NotSaving(message.clone()));
        }
        if let Some(id) = &self.diverged {
            return Some(Alert::Diverged(id.clone()));
        }
        self.vanished.clone().map(Alert::Vanished)
    }

    fn clear_alerts(&mut self) {
        self.not_saving = None;
        self.diverged = None;
        self.vanished = None;
    }

    /// Act on whatever the banner is offering. `false` when there is nothing to
    /// do, which is the save-failure case: the next tick retries by itself.
    pub fn resolve_alert(&mut self) -> bool {
        match self.alert() {
            Some(Alert::Diverged(_)) => self.take_disk_version(),
            Some(Alert::Vanished(_)) => self.restore_open_note(),
            Some(Alert::NotSaving(_)) | None => false,
        }
    }

    /// Something changed the vault from outside. Take it on.
    pub fn absorb_external_changes(&mut self) -> External {
        self.rescan();

        let Some(id) = self.open.clone() else {
            return External::Quiet;
        };
        if !self.index.contains(&id) {
            self.vanished = Some(id.clone());
            return External::Vanished { id };
        }
        // It came back — restored from the trash, or a `git checkout` that
        // undid the delete.
        self.vanished = None;
        let Some(vault) = self.vault.clone() else {
            return External::Quiet;
        };
        let Ok(note) = vault.read(&id) else {
            return External::Quiet;
        };
        let text = note.to_text();

        // An event is not a change. A watcher fires for Brain's own saves as
        // loudly as for anyone else's, so what counts is whether the file
        // differs from what Brain last put there or read from there —
        // otherwise typing raises a "changed on disk" warning every two
        // seconds, about your own keystrokes.
        if self.on_disk.as_deref() == Some(text.as_str()) {
            return External::Quiet;
        }
        self.on_disk = Some(text.clone());

        if self.dirty {
            self.diverged = Some(id.clone());
            return External::Diverged { id, on_disk: text };
        }

        // Nothing unsaved, so the file is the truth.
        self.buffer = Some(note);
        self.diverged = None;
        External::Reloaded
    }

    /// Take the version on disk and drop the unsaved local edits.
    ///
    /// The counterpart to keeping them is doing nothing: the buffer already
    /// holds what was typed, and the next save writes it over the file.
    pub fn take_disk_version(&mut self) -> bool {
        let Some(id) = self.open.clone() else {
            return false;
        };
        let Some(vault) = self.vault.clone() else {
            return false;
        };
        let Ok(note) = vault.read(&id) else {
            return false;
        };
        self.on_disk = Some(note.to_text());
        self.buffer = Some(note);
        self.dirty = false;
        self.diverged = None;
        true
    }

    /// Write the open note back to a path it has disappeared from.
    ///
    /// The counterpart to [`Self::take_disk_version`] for a note deleted
    /// outside Brain: the editor is holding the only copy left, and without
    /// this the only way to get it back on disk is to type a character and
    /// wait for the tick, which nobody would guess at.
    pub fn restore_open_note(&mut self) -> bool {
        let (Some(vault), Some(note)) = (self.vault.clone(), self.buffer.clone()) else {
            return false;
        };
        if vault.write(&note).is_err() {
            return false;
        }
        self.on_disk = Some(note.to_text());
        self.index.update(&note);
        self.dirty = false;
        self.vanished = None;
        true
    }

    // ---- vectors ----

    /// Point the vector store at the current vault, reading whatever was cached.
    fn load_vectors(&mut self) {
        let Some(vault) = self.vault.clone() else {
            self.vectors = semantic::Store::default();
            self.vectors_path = None;
            return;
        };
        let path = semantic::default_store_path(vault.root());
        self.vectors = semantic::Store::load(&path);
        self.vectors_path = Some(path);
    }

    /// The index and store a catch-up pass should run against.
    pub fn catch_up_input(&self) -> (Index, semantic::Store) {
        (self.index.clone(), self.vectors.clone())
    }

    /// Take on the store a pass produced, and write it out.
    ///
    /// A cache that could not be written is not worth a word to the user: the
    /// vectors are in memory and work, and the next pass writes them again.
    pub fn absorb_vectors(&mut self, store: semantic::Store) {
        self.vectors = store;
        if let Some(path) = self.vectors_path.clone() {
            let _ = self.vectors.save(&path);
        }
    }

    /// The vectors as they stand, for tests and for callers that want to search
    /// the vault themselves.
    pub fn vectors(&self) -> semantic::Store {
        self.vectors.clone()
    }

    pub fn has_vectors(&self) -> bool {
        !self.vectors.is_empty()
    }

    /// Remember the vector for a query, so the next search can fuse with it.
    pub fn set_query_vector(&mut self, query: String, vector: Vec<f32>) {
        self.query_vector = Some((query, vector));
    }

    // ---- what the sidebar shows ----

    /// Whether the sidebar is showing results rather than the tree.
    pub fn is_searching(&self) -> bool {
        !self.query.trim().is_empty()
    }

    /// The ids the tag filter leaves, before any search.
    fn tagged_ids(&self) -> Vec<NoteId> {
        match self.filter.as_ref() {
            Some(tag) => self.index.notes_tagged(tag),
            None => self.index.ids().cloned().collect(),
        }
    }

    /// The notes the sidebar is showing: every note, or the ones carrying the
    /// active tag, or the ones matching the search — in path order so folders
    /// group and nothing ever reshuffles, unless a search has ranked them.
    pub fn listed_notes(&self) -> Vec<(NoteId, String)> {
        if self.is_searching() {
            return self.search_results();
        }
        let mut notes: Vec<(NoteId, String)> = self
            .tagged_ids()
            .into_iter()
            .map(|id| {
                let excerpt = self.index.excerpt(&id).to_string();
                (id, excerpt)
            })
            .collect();
        notes.sort_by(|a, b| a.0.cmp(&b.0));
        notes
    }

    /// What the sidebar search matched, best first.
    ///
    /// Titles before text, because someone typing into a list of notes is
    /// usually reaching for one they can name. A note matched by both appears
    /// once, at its title rank, with the matching line as its excerpt so the
    /// row says *why* it is there.
    pub fn search_results(&self) -> Vec<(NoteId, String)> {
        let query = self.query.trim().to_string();
        if query.is_empty() {
            return Vec::new();
        }
        let allowed: BTreeSet<NoteId> = self.tagged_ids().into_iter().collect();

        let mut out: Vec<(NoteId, String)> = Vec::new();
        for matched in search::by_title(&self.index, &query, 50) {
            if allowed.contains(&matched.id) {
                out.push((
                    matched.id.clone(),
                    self.index.excerpt(&matched.id).to_string(),
                ));
            }
        }
        for matched in search::by_text(&self.index, &query, 50) {
            if !allowed.contains(&matched.id) || out.iter().any(|(id, _)| id == &matched.id) {
                continue;
            }
            let excerpt = matched
                .snippets
                .first()
                .map(|snippet| snippet.text.clone())
                .unwrap_or_default();
            out.push((matched.id.clone(), excerpt));
        }
        out
    }

    /// The sidebar's tree, folders and all.
    pub fn sidebar_rows(&self) -> Vec<Row> {
        let notes: Vec<Listed> = self
            .tagged_ids()
            .into_iter()
            .map(|id| {
                // Timestamps are read from disk only when something is going to
                // sort by them. By name — the default — the sidebar refreshes
                // on every save, and stat-ing the vault each time to sort by a
                // field nobody asked for is work for nothing.
                let (modified, created) = match (self.sort, &self.vault) {
                    (Sort::Name, _) | (_, None) => (0, 0),
                    (_, Some(vault)) => vault.times(&id),
                };
                Listed {
                    excerpt: self.index.excerpt(&id).to_string(),
                    id,
                    modified,
                    created,
                }
            })
            .collect();

        // A tag filter is a question about notes, not about folders. So the
        // empty folders drop out of the tree while one is on rather than
        // sitting there claiming to hold something, and every folder that does
        // hold a match is opened — a filter whose results are behind a chevron
        // is a filter that looks like it found nothing.
        let filtered = self.filter.is_some();
        let folders = match (filtered, &self.vault) {
            (false, Some(vault)) => vault.folders(),
            _ => Vec::new(),
        };
        let expanded = if filtered {
            notes
                .iter()
                .flat_map(|note| ancestors(note.id.folder().unwrap_or("")))
                .collect()
        } else {
            self.expanded.clone()
        };
        tree::rows(&notes, &folders, &expanded, self.sort)
    }

    /// Every tag in the vault, with its note count.
    pub fn tags(&self) -> Vec<(String, usize)> {
        self.index.tags()
    }

    pub fn active_tag(&self) -> Option<String> {
        self.filter.clone()
    }

    /// Show only notes carrying `tag`, or all of them.
    ///
    /// A tag that no longer exists — the last note carrying it was retagged —
    /// clears the filter rather than showing an empty list with no way out.
    /// `false` means exactly that happened.
    pub fn filter_by_tag(&mut self, tag: Option<&str>) -> bool {
        let wanted = tag.map(|tag| tag.trim_start_matches('#').to_lowercase());
        let known = wanted
            .as_ref()
            .is_some_and(|tag| !self.index.notes_tagged(tag).is_empty());
        self.filter = if known { wanted } else { None };
        known || tag.is_none()
    }

    /// Open or close a folder, and make it where the next new note goes.
    pub fn toggle_folder(&mut self, path: &str) {
        if !self.expanded.remove(path) {
            self.expanded.insert(path.to_string());
        }
        self.target_folder = Some(path.to_string());
    }

    /// Open every folder on the way to this one, so a note revealed by a search
    /// or a link is visible in the tree behind it.
    fn expand_to(&mut self, id: &NoteId) {
        let Some(folder) = id.folder() else {
            return;
        };
        for ancestor in ancestors(folder) {
            self.expanded.insert(ancestor);
        }
    }

    pub fn sort(&self) -> Sort {
        self.sort
    }

    pub fn set_sort(&mut self, sort: Sort) {
        self.sort = sort;
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// Filter the sidebar by what was typed into its search entry. `false` when
    /// nothing changed, so the shell can skip the rebuild.
    pub fn set_query(&mut self, query: &str) -> bool {
        if self.query == query {
            return false;
        }
        self.query = query.to_string();
        true
    }

    // ---- the open note ----

    pub fn open_note_id(&self) -> Option<NoteId> {
        self.open.clone()
    }

    /// The open note's full text, frontmatter included.
    ///
    /// The editor holds the *file*: the design styles that block in place, and
    /// a note whose metadata is only reachable through some other pane is a
    /// note with a hidden half. Round-tripping it costs nothing — an untouched
    /// block is written back byte for byte.
    pub fn open_note_text(&self) -> Option<(NoteId, String)> {
        match (&self.open, &self.buffer) {
            (Some(id), Some(note)) => Some((id.clone(), note.to_text())),
            _ => None,
        }
    }

    pub fn open_note(&self) -> Option<&Note> {
        self.buffer.as_ref()
    }

    /// Read a note into the editor. Switching notes is the caller's cue to
    /// flush and save the one being left, rather than waiting for the tick and
    /// racing it.
    pub fn load_note(&mut self, id: &NoteId) -> Result<(), VaultError> {
        let Some(vault) = self.vault.clone() else {
            return Ok(());
        };
        // Every banner condition is about the note being left, so none of them
        // survives opening another one.
        self.clear_alerts();
        match vault.read(id) {
            Ok(note) => {
                self.on_disk = Some(note.to_text());
                self.open = Some(id.clone());
                self.buffer = Some(note);
                self.config.last_note = Some(id.as_str().to_string());
                self.target_folder = Some(id.folder().unwrap_or("").to_string());
                // Opening a note the tree cannot show — from a link, a search
                // result, or the last session — reveals it rather than leaving
                // the highlight inside a folder that is shut.
                self.expand_to(id);
                Ok(())
            }
            Err(error) => {
                self.open = None;
                self.buffer = None;
                self.on_disk = None;
                Err(error)
            }
        }
    }

    /// Reopen whatever was open when Brain last closed.
    pub fn restore_last_note(&mut self) -> Option<NoteId> {
        let last = self.config.last_note.clone()?;
        let id = NoteId::from_relative(last);
        if !self.index.contains(&id) {
            return None;
        }
        self.load_note(&id).ok()?;
        Some(id)
    }

    /// The editor's text changed.
    pub fn mark_edited(&mut self) {
        self.dirty = true;
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Copy the editor's text into the open note. Does not write to disk.
    ///
    /// Kept separate from [`Self::save_now`] because the two have different
    /// callers: every save flushes first, but the shell also flushes when it is
    /// about to hand control somewhere the editor may go away.
    pub fn flush(&mut self, body: &str) {
        let Some(note) = self.buffer.as_mut() else {
            return;
        };
        // The editor holds the whole file, so the frontmatter is re-split out
        // of it rather than carried alongside.
        let edited = Note::from_text(note.id.clone(), body);
        if edited != *note {
            *note = edited;
            self.dirty = true;
        }
    }

    /// Write the open note if it has unsaved changes.
    pub fn save_now(&mut self) -> Saved {
        if !self.dirty {
            return Saved::Clean;
        }
        let (Some(vault), Some(note)) = (self.vault.clone(), self.buffer.clone()) else {
            self.dirty = false;
            return Saved::Clean;
        };
        match vault.write(&note) {
            Ok(()) => {
                self.dirty = false;
                self.on_disk = Some(note.to_text());
                self.index.update(&note);
                // A successful write settles all three: saving is evidently
                // working, the file exists, and a divergence has just been
                // decided in favour of what was on screen.
                self.not_saving = None;
                self.diverged = None;
                self.vanished = None;
                Saved::Written
            }
            Err(error) => {
                self.not_saving = Some(error.to_string());
                Saved::Failed(error)
            }
        }
    }

    // ---- links and search ----

    /// Candidates for a `[[` completion, by title, alias and path.
    pub fn link_candidates(&self, query: &str) -> Vec<String> {
        search::by_title(&self.index, query, 16)
            .into_iter()
            // Linking a note to itself is never what was meant.
            .filter(|matched| Some(&matched.id) != self.open.as_ref())
            .map(|matched| matched.id.title().to_string())
            .collect()
    }

    /// What a `[[link]]` points at. The shell decides whether that means open
    /// it, offer to write it, or ask which of two was meant.
    pub fn resolve_link(&self, target: &str) -> Resolution {
        self.index.resolve(target, self.open.as_ref())
    }

    /// The notes linking to the open one, each with the line it was linked from.
    pub fn backlinks_of_open_note(&self) -> Vec<(NoteId, String)> {
        let Some(id) = self.open.as_ref() else {
            return Vec::new();
        };
        self.index
            .backlinks(id)
            .iter()
            .map(|backlink| (backlink.from.clone(), backlink.context.clone()))
            .collect()
    }

    /// Answer a palette query.
    ///
    /// In text mode this is hybrid: BM25 over the words, vectors over the
    /// meaning, fused. The vector half is whatever is already known about this
    /// query — on the first keystroke that is nothing, so the first answer is
    /// lexical and the fused one replaces it a moment later. `wants_embedding`
    /// says the shell should go and get that vector.
    pub fn search(&self, query: &str, mode: Mode) -> (Vec<Hit>, bool) {
        match mode {
            Mode::Title => {
                let hits = search::by_title(&self.index, query, 30)
                    .into_iter()
                    .map(|matched| Hit {
                        id: matched.id.as_str().to_string(),
                        title: matched.id.title().to_string(),
                        // The folder, since two notes can share a title and the
                        // path is the only thing telling them apart.
                        detail: matched.id.folder().unwrap_or("").to_string(),
                        highlight: None,
                    })
                    .collect();
                (hits, false)
            }
            Mode::Text => {
                let embedded = self
                    .query_vector
                    .as_ref()
                    .filter(|(embedded, _)| embedded == query)
                    .map(|(_, vector)| (&self.vectors, vector.as_slice()));
                let wants_embedding = embedded.is_none();

                let hits = search::hybrid(&self.index, &self.lexical, embedded, query, 30)
                    .into_iter()
                    .map(|hit| Hit {
                        id: hit.id.as_str().to_string(),
                        title: hit.id.title().to_string(),
                        // Underlined where the words are, and not underlined at
                        // all when the vectors found it and the words are not
                        // there — which is the honest rendering of a hit that
                        // matched on meaning.
                        highlight: search::highlight_of(&hit.snippet, query),
                        detail: hit.snippet,
                    })
                    .collect();
                (hits, wants_embedding)
            }
        }
    }

    // ---- attachments ----

    /// Copy files into the vault. The names come back for the shell to embed.
    pub fn attach_files(&mut self, paths: &[String]) -> Attached {
        let Some(vault) = self.vault.clone() else {
            return Attached::default();
        };
        let mut attached = Attached::default();
        for path in paths {
            match vault.add_attachment(Path::new(path)) {
                Ok(name) => attached.names.push(name),
                Err(_) => attached.failures += 1,
            }
        }
        attached
    }

    /// Files in `attachments/` that no note refers to.
    pub fn unused_attachments(&self) -> Vec<String> {
        let Some(vault) = self.vault.as_ref() else {
            return Vec::new();
        };
        let referenced = self.index.referenced_attachments();
        let directory = vault.root().join(crate::vault::ATTACHMENTS_DIR);
        let mut unused: Vec<String> = std::fs::read_dir(&directory)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| !name.starts_with('.') && !referenced.contains(name))
            .collect();
        unused.sort();
        unused
    }

    // ---- creating, renaming, deleting ----

    /// Where a new note or folder goes: whichever folder was last chosen in the
    /// sidebar, and otherwise beside the open note, so working inside a folder
    /// keeps you there.
    pub fn current_folder(&self) -> String {
        self.target_folder
            .clone()
            .or_else(|| {
                self.open
                    .as_ref()
                    .and_then(|id| id.folder().map(str::to_string))
            })
            .unwrap_or_default()
    }

    /// Write a new note in a named folder and open it.
    pub fn create_note_in(&mut self, folder: &str, title: &str) -> Result<NoteId, Failed> {
        let Some(vault) = self.vault.clone() else {
            return Err(Failed::NoVault);
        };
        let id = unique_id(&vault, Some(folder), title);
        let note = vault.create(&id, "")?;
        self.index.update(&note);
        self.load_note(&id)?;
        Ok(id)
    }

    /// Rename the open note and repoint every link that pointed at it.
    pub fn rename_note(&mut self, title: &str) -> Renamed {
        let (Some(vault), Some(from)) = (self.vault.clone(), self.open.clone()) else {
            return Renamed::Unchanged;
        };
        if from.title() == title {
            return Renamed::Unchanged;
        }
        let to = match from.folder() {
            Some(folder) => NoteId::from_relative(format!("{folder}/{title}.md")),
            None => NoteId::from_relative(format!("{title}.md")),
        };

        // Every note pointing here has to be rewritten, so gather them before
        // the rename makes the index forget who they were.
        let inbound: Vec<NoteId> = self
            .index
            .backlinks(&from)
            .iter()
            .map(|backlink| backlink.from.clone())
            .collect();

        if let Err(error) = vault.rename(&from, &to) {
            return Renamed::Failed(error);
        }
        self.index.rename(&from, &to);

        let mut links = 0usize;
        for id in inbound {
            let Ok(mut note) = vault.read(&id) else {
                continue;
            };
            let Some(body) = markdown::rewrite_target(&note.body, from.title(), to.title()) else {
                continue;
            };
            note.body = body;
            if vault.write(&note).is_ok() {
                self.index.update(&note);
                links += 1;
            }
        }

        self.open = Some(to.clone());
        if let Some(note) = self.buffer.as_mut() {
            note.id = to.clone();
        }
        self.config.last_note = Some(to.as_str().to_string());
        Renamed::Done { to, links }
    }

    /// Delete the open note. The pending write is dropped first, or the tick
    /// recreates the file.
    pub fn delete_open_note(&mut self) -> Result<NoteId, Failed> {
        let (Some(vault), Some(id)) = (self.vault.clone(), self.open.clone()) else {
            return Err(Failed::NoVault);
        };
        self.dirty = false;
        self.buffer = None;
        self.on_disk = None;
        self.open = None;
        self.clear_alerts();

        vault.delete(&id)?;
        self.index.remove(&id);
        self.config.last_note = None;
        Ok(id)
    }

    // ---- folders ----

    /// The folders in the vault, for the dialogs that ask which one.
    pub fn folders(&self) -> Vec<String> {
        self.vault.as_ref().map(Vault::folders).unwrap_or_default()
    }

    pub fn create_folder(&mut self, parent: &str, name: &str) -> Result<String, Failed> {
        let Some(vault) = self.vault.clone() else {
            return Err(Failed::NoVault);
        };
        let path = join(parent, name);
        vault.create_folder(&path)?;
        // Opened, and opened all the way down to it: a folder made and then not
        // shown is a folder you go looking for in Files.
        for ancestor in ancestors(&path) {
            self.expanded.insert(ancestor);
        }
        self.target_folder = Some(path.clone());
        Ok(path)
    }

    pub fn delete_folder(&mut self, path: &str) -> Result<(), Failed> {
        let Some(vault) = self.vault.clone() else {
            return Err(Failed::NoVault);
        };
        vault.delete_folder(path)?;
        self.expanded.remove(path);
        // Drop a remembered target folder that has just stopped existing.
        let stale = self
            .target_folder
            .as_deref()
            .is_some_and(|target| tree::is_within(path, target));
        if stale {
            self.target_folder = None;
        }
        Ok(())
    }

    /// Rename a folder in place, keeping everything under it.
    pub fn rename_folder(&mut self, path: &str, name: &str) -> Result<(), Failed> {
        let parent = path
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .unwrap_or("");
        self.relocate_folder(path, &join(parent, name))
    }

    /// Move or rename a folder, and take the notebook's own state with it.
    ///
    /// The index is rebuilt rather than patched: every note under the folder
    /// changes id at once, and walking a subtree of the index to rewrite ids is
    /// exactly the kind of bookkeeping a rescan does correctly for free.
    pub fn relocate_folder(&mut self, from: &str, to: &str) -> Result<(), Failed> {
        let Some(vault) = self.vault.clone() else {
            return Err(Failed::NoVault);
        };
        if from == to {
            return Ok(());
        }
        vault.move_folder(from, to)?;

        // Anything that named a path under the old folder now names nothing.
        let moved = |path: &str| -> Option<String> {
            if path == from {
                Some(to.to_string())
            } else {
                path.strip_prefix(&format!("{from}/"))
                    .map(|rest| format!("{to}/{rest}"))
            }
        };
        self.expanded = self
            .expanded
            .iter()
            .map(|path| moved(path).unwrap_or_else(|| path.clone()))
            .collect();
        for ancestor in ancestors(to) {
            self.expanded.insert(ancestor);
        }
        if let Some(target) = self.target_folder.clone() {
            self.target_folder = Some(moved(&target).unwrap_or(target));
        }

        let reopen = self
            .open
            .clone()
            .and_then(|id| moved(id.as_str()).map(NoteId::from_relative));

        self.rescan();
        if let Some(id) = reopen {
            self.config.last_note = Some(id.as_str().to_string());
            let _ = self.load_note(&id);
        }
        Ok(())
    }

    /// Move one note into a folder, keeping its title.
    ///
    /// Inbound links are left alone deliberately: they resolve by title, and
    /// the title has not changed. A move is the one rearrangement of the vault
    /// that costs nothing anywhere else — which is the point of folders being
    /// organisational rather than namespaces.
    pub fn move_note(&mut self, id: &NoteId, destination: &str) -> Moved {
        let Some(vault) = self.vault.clone() else {
            return Moved::Unchanged;
        };
        if id.folder().unwrap_or("") == destination {
            return Moved::Unchanged;
        }
        let to = NoteId::from_relative(join(destination, &format!("{}.md", id.title())));
        let was_open = self.open.as_ref() == Some(id);

        if let Err(error) = vault.rename(id, &to) {
            return Moved::Failed(error);
        }
        self.index.rename(id, &to);

        if was_open {
            self.open = Some(to.clone());
            if let Some(note) = self.buffer.as_mut() {
                note.id = to.clone();
            }
            self.config.last_note = Some(to.as_str().to_string());
        }
        self.expand_to(&to);
        Moved::Done {
            to,
            destination: destination.to_string(),
        }
    }
}

/// A free id for `title`, suffixed if that name is taken.
fn unique_id(vault: &Vault, folder: Option<&str>, title: &str) -> NoteId {
    let build = |name: &str| match folder {
        Some(folder) if !folder.is_empty() => NoteId::from_relative(format!("{folder}/{name}.md")),
        _ => NoteId::from_relative(format!("{name}.md")),
    };
    let mut id = build(title);
    let mut attempt = 2;
    while vault.path_of(&id).exists() {
        id = build(&format!("{title} {attempt}"));
        attempt += 1;
    }
    id
}

/// `parent` and `name` as one vault-relative path. An empty parent is the vault
/// root, where a leading `/` would name something outside it.
fn join(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

/// `a/b/c` yields `a`, `a/b`, `a/b/c`.
fn ancestors(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for segment in path.split('/').filter(|segment| !segment.is_empty()) {
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(segment);
        out.push(current.clone());
    }
    out
}
