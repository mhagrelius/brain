//! Deciding what to do when a vault exists in more than one place.
//!
//! A pure function, the same shape as [`crate::semantic::plan`] and for the
//! same reason: nothing subscribes to anything. Take what this machine holds
//! now, what the server holds now, and what the two agreed on last time, and
//! return work. A note edited in Brain, a folder moved in Nautilus, and a
//! machine that was switched off for a week are the same input.
//!
//! # Three snapshots, not two
//!
//! Comparing local against remote can tell you they differ but never which of
//! them moved, and "differs" is not enough to act on — pushing when you should
//! have pulled is how a sync loses a note. So a third snapshot is kept: what
//! the two sides agreed on at the end of the last pass. Local against base
//! says whether *this* machine changed something; remote against base says
//! whether another one did. Only when both did is there anything to resolve.
//!
//! # The two rules that are rules rather than questions
//!
//! **A deletion never wins over an edit.** If one machine deleted a note and
//! another edited it, the note survives. Losing a note is the one
//! unrecoverable failure here, and the same reasoning already applies to
//! [`crate::semantic`]'s stale vectors: the recoverable mistake is always the
//! one to make.
//!
//! **A rename and an edit compose.** Brain renames by writing a new path and
//! removing the old one, so from here a rename looks like a delete beside a
//! create. It is told apart the way a move is told apart in `semantic::plan` —
//! same content hash under a new id — and the two changes are then applied
//! one after the other rather than fought over.
//!
//! # Everything else is a note, not a dialog
//!
//! A genuine conflict — both sides edited the same note into different text —
//! is resolved by keeping this machine's version where it is and writing the
//! other one into the vault beside it under a name that says where it came
//! from. It survives Brain being closed, crashed, or uninstalled, which a
//! dialog answer does not; it appears in the sidebar because the sidebar is
//! the directory tree; and it is resolved with what already exists — open
//! both, reconcile, delete the loser, which carries undo.

use std::collections::{BTreeMap, BTreeSet};

use crate::note::{Note, NoteId};
use crate::vault::Vault;

/// A fingerprint of a note's bytes.
///
/// Distinct from [`crate::semantic::Digest`], which is over the title and the
/// *stripped* text because that is what a model reads. This is over the file,
/// so a change to frontmatter that no model would notice still syncs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hash(pub u64);

impl Hash {
    pub fn of(text: &str) -> Self {
        // FNV-1a. The same choice `semantic::Digest` made and for the same
        // reasons: no dependency, stable across runs and machines, and this is
        // change detection rather than anything anyone is attacking.
        let mut hash: u64 = 0xcbf29ce484222325;
        for byte in text.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        Self(hash)
    }
}

/// What one side holds: every note it has, by id, with the hash of its bytes.
pub type Snapshot = BTreeMap<NoteId, Hash>;

/// Both sides edited the same note into different text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// The note as it stands here. It is left exactly where it is — the
    /// machine you are typing on does not have its work moved out from under
    /// it to make room for somebody else's.
    pub id: NoteId,
    /// Where the other side's version is written, beside it.
    pub copy: NoteId,
}

/// The work one pass should do.
///
/// Empty in every field is the steady state and worth asserting on: a plan
/// that is not empty when nothing changed means something is re-uploading the
/// vault on a timer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// Send this machine's version up.
    pub push: Vec<NoteId>,
    /// Take the server's version down.
    pub pull: Vec<NoteId>,
    /// Remove locally, because another machine deleted it and nothing here
    /// disagreed.
    pub delete_local: Vec<NoteId>,
    /// Remove on the server, because this machine deleted it and nothing there
    /// disagreed.
    pub delete_remote: Vec<NoteId>,
    /// A rename another machine made, to apply here. `fs::rename`, not a read
    /// and a write: the content is already right.
    pub rename_local: Vec<(NoteId, NoteId)>,
    /// Both sides changed the same note. See [`Conflict`].
    pub conflicts: Vec<Conflict>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// How many notes this pass touches, for the "3 notes had conflicting
    /// edits" sort of message.
    pub fn len(&self) -> usize {
        self.push.len()
            + self.pull.len()
            + self.delete_local.len()
            + self.delete_remote.len()
            + self.rename_local.len()
            + self.conflicts.len()
    }
}

/// Where the other side's version of a conflicted note is written.
///
/// The name carries the provenance rather than the frontmatter, which keeps
/// the note format frozen at its four keys. A different title also means the
/// copy cannot steal `[[Rust ownership]]` links or trip the same-title
/// ambiguity report — it is a note about a note, not a second answer to the
/// same name.
pub fn conflict_id(id: &NoteId, from: &str, date: &str) -> NoteId {
    let stem = format!("{} (conflict {date} from {from})", id.title());
    match id.folder() {
        Some(folder) if !folder.is_empty() => NoteId::from_relative(format!("{folder}/{stem}.md")),
        _ => NoteId::from_relative(format!("{stem}.md")),
    }
}

/// Decide what one pass should do.
///
/// `base` is what the two sides agreed on at the end of the last pass. An
/// empty one is a first pass, where everything on both sides is new to the
/// other and nothing can be a conflict that is not a genuine one.
pub fn plan(base: &Snapshot, local: &Snapshot, remote: &Snapshot, from: &str, date: &str) -> Plan {
    let mut plan = Plan::default();

    // A rename is a delete beside a create with the same content, told apart
    // exactly the way `semantic::plan` tells a move apart. Detected first,
    // because every id it accounts for must not then be read as a deletion.
    let local_renames = renames(base, local);
    let remote_renames = renames(base, remote);
    let mut settled: BTreeSet<NoteId> = BTreeSet::new();
    for (from_id, to_id) in &remote_renames {
        // A rename here and there of the same note to the same place is not a
        // disagreement, and to different places is one this does not try to
        // be clever about — the id comparison below catches it.
        if local_renames.iter().any(|(old, _)| old == from_id) {
            continue;
        }
        plan.rename_local.push((from_id.clone(), to_id.clone()));
        settled.insert(from_id.clone());
        settled.insert(to_id.clone());
    }
    for (from_id, to_id) in &local_renames {
        settled.insert(from_id.clone());
        settled.insert(to_id.clone());
        // The other side has not seen this one yet, so its new name goes up.
        if !remote.contains_key(to_id) {
            plan.push.push(to_id.clone());
        }
        if remote.contains_key(from_id) {
            plan.delete_remote.push(from_id.clone());
        }
    }

    let ids: BTreeSet<&NoteId> = base
        .keys()
        .chain(local.keys())
        .chain(remote.keys())
        .collect();

    for id in ids {
        if settled.contains(id) {
            continue;
        }
        let (was, here, there) = (base.get(id), local.get(id), remote.get(id));
        let changed_here = here != was;
        let changed_there = there != was;

        match (changed_here, changed_there) {
            (false, false) => {}
            (true, false) => match here {
                Some(_) => plan.push.push(id.clone()),
                None => plan.delete_remote.push(id.clone()),
            },
            (false, true) => match there {
                Some(_) => plan.pull.push(id.clone()),
                None => plan.delete_local.push(id.clone()),
            },
            (true, true) => match (here, there) {
                // Both arrived at the same text. Nothing to do but agree.
                (Some(a), Some(b)) if a == b => {}
                // A deletion never wins over an edit. Whichever side still has
                // the note is the one that is right, and the other side takes
                // it back.
                (None, Some(_)) => plan.pull.push(id.clone()),
                (Some(_), None) => plan.push.push(id.clone()),
                // Deleted on both sides, which is agreement, not conflict.
                (None, None) => {}
                // Genuinely different text on both sides. Neither is thrown
                // away.
                (Some(_), Some(_)) => plan.conflicts.push(Conflict {
                    id: id.clone(),
                    copy: conflict_id(id, from, date),
                }),
            },
        }
    }
    plan
}

/// Why a pass could not finish. One string, like [`crate::semantic::EmbedError`]
/// and for the same reason: every caller's recovery is to leave the vault alone
/// and try again, so a taxonomy would only give them something to ignore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncError(pub String);

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a write to the server did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Put {
    Done(Hash),
    /// The server moved on since the base this was sent with. Carries what it
    /// actually holds, which is what turns a refusal into a plannable conflict
    /// rather than a loop.
    Stale(Option<Hash>),
    Failed(String),
}

/// The other side of the vault, over whatever the shell speaks.
///
/// The same arrangement as [`crate::semantic::Embedder`]: synchronous, because
/// the caller is already on a worker thread, and a trait so the whole of the
/// pass below can be tested against a map instead of a network.
pub trait Remote {
    fn list(&self) -> Result<Snapshot, SyncError>;
    /// The text of each id the server still has. Missing ones are absent.
    fn get(&self, ids: &[NoteId]) -> Result<Vec<(NoteId, String)>, SyncError>;
    fn put(&self, id: &NoteId, text: &str, base: Option<Hash>) -> Put;
    fn delete(&self, id: &NoteId, base: Option<Hash>) -> Put;
}

/// What one pass did, for the banner and for the tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub pushed: usize,
    pub pulled: usize,
    pub deleted_here: usize,
    pub deleted_there: usize,
    pub renamed: usize,
    /// Notes that ended up with a copy beside them. The number the banner says.
    pub conflicted: usize,
    /// Transfers that did not happen. Not an error — the next pass retries,
    /// and the base was not advanced for them, so nothing is forgotten.
    pub failed: usize,
}

impl Report {
    /// Whether anything happened. A quiet vault reports nothing at all, which
    /// is what makes it safe to run after every rescan.
    pub fn is_quiet(&self) -> bool {
        *self == Self::default()
    }
}

/// Notes that are gone from `after` but whose content turned up under a new id.
fn renames(base: &Snapshot, after: &Snapshot) -> Vec<(NoteId, NoteId)> {
    // Ids that are new in `after`, indexed by content, so a vanished note can
    // be looked for by what it said.
    let mut arrived: BTreeMap<Hash, Vec<&NoteId>> = BTreeMap::new();
    for (id, hash) in after {
        if !base.contains_key(id) {
            arrived.entry(*hash).or_default().push(id);
        }
    }

    let mut found = Vec::new();
    let mut taken: BTreeSet<&NoteId> = BTreeSet::new();
    for (id, hash) in base {
        if after.contains_key(id) {
            continue;
        }
        // Two notes with identical text renamed at once cannot be told apart,
        // and guessing which became which would rename them into each other's
        // places. The first free candidate is as good as any and the content
        // is the same either way.
        let Some(candidates) = arrived.get(hash) else {
            continue;
        };
        if let Some(to) = candidates.iter().find(|to| !taken.contains(**to)) {
            taken.insert(to);
            found.push((id.clone(), (*to).clone()));
        }
    }
    found
}

/// Take a snapshot of what is on disk right now.
pub fn snapshot_of(vault: &Vault) -> Snapshot {
    let (notes, _problems) = vault.scan();
    notes
        .into_iter()
        .map(|note| (note.id.clone(), Hash::of(&note.to_text())))
        .collect()
}

/// One note the server sent, and where it should land here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Landing {
    /// Where to write it. For a conflict this is the copy, never the original.
    pub id: NoteId,
    pub text: String,
    /// The note this is the other side's version of, when it is a conflict
    /// copy. `None` for an ordinary pull.
    pub conflict_with: Option<NoteId>,
}

/// What one pass got off the network, ready to be written locally.
///
/// The whole point of this type is that producing it touches no local file.
/// See [`gather`].
#[derive(Debug, Clone, Default)]
pub struct Incoming {
    /// Notes to write here, ordinary pulls and conflict copies alike.
    pub land: Vec<Landing>,
    pub rename: Vec<(NoteId, NoteId)>,
    pub delete: Vec<NoteId>,
    /// The base as the network half left it: what was pushed and what was
    /// deleted there. [`apply`] adds what the local half manages.
    pub agreed: Snapshot,
    /// What the network half already did.
    pub report: Report,
}

/// The network half of a pass. **Reads local files; writes none.**
///
/// This split is the whole reason a sync is safe on a worker thread, and it is
/// not the shape it first looks like it should be. The embedding catch-up can
/// do its work off the main loop because it is handed copies and gives back a
/// new store — nothing shared, nothing to race. A sync writes *files*, and the
/// filesystem is shared with the save tick: a pull landing on a note the user
/// edited two seconds ago would overwrite it, and the server's stale-write
/// check is no help because it guards the server's copy, not this one.
///
/// So the worker does this, which only ever reads, and every local write
/// happens in [`apply`] on the thread that owns the notebook and therefore
/// knows which note is open and whether it is dirty.
///
/// A push sending text that has changed since the snapshot is not a problem:
/// the base records what the server took, so the next pass sees local differ
/// from it and pushes again. It converges, and no version is lost on the way.
pub fn gather(
    vault: &Vault,
    base: &Snapshot,
    remote: &dyn Remote,
    from: &str,
    date: &str,
) -> Result<Incoming, SyncError> {
    let local = snapshot_of(vault);
    let there = remote.list()?;
    let plan = plan(base, &local, &there, from, date);

    let mut incoming = Incoming {
        agreed: base.clone(),
        ..Incoming::default()
    };

    // Renames are carried, not applied: `fs::rename` is a write.
    incoming.rename = plan.rename_local.clone();
    incoming.delete = plan.delete_local.clone();

    // Conflicts are fetched like anything else. Where they land is what
    // differs, and that is [`Landing`]'s job.
    let landing: BTreeMap<&NoteId, &NoteId> = plan
        .conflicts
        .iter()
        .map(|conflict| (&conflict.id, &conflict.copy))
        .collect();
    let pulling: Vec<NoteId> = plan
        .pull
        .iter()
        .cloned()
        .chain(plan.conflicts.iter().map(|conflict| conflict.id.clone()))
        .collect();

    for (id, text) in remote.get(&pulling)? {
        let copy = landing.get(&id).copied();
        incoming.land.push(Landing {
            id: copy.cloned().unwrap_or_else(|| id.clone()),
            text,
            conflict_with: copy.map(|_| id),
        });
    }

    for id in &plan.push {
        let Some(text) = read_text(vault, id) else {
            incoming.report.failed += 1;
            continue;
        };
        match remote.put(id, &text, base.get(id).copied()) {
            Put::Done(hash) => {
                incoming.agreed.insert(id.clone(), hash);
                incoming.report.pushed += 1;
            }
            // Somebody wrote between the listing and this. Not an error and
            // not a conflict yet — the next pass lists again and sees it.
            Put::Stale(_) | Put::Failed(_) => incoming.report.failed += 1,
        }
    }

    // Deletions last, because a deletion is the only step the next pass cannot
    // undo: a failure before this point leaves the note in place rather than
    // gone.
    for id in &plan.delete_remote {
        match remote.delete(id, base.get(id).copied()) {
            Put::Done(_) => {
                incoming.agreed.remove(id);
                incoming.report.deleted_there += 1;
            }
            Put::Stale(_) | Put::Failed(_) => incoming.report.failed += 1,
        }
    }

    Ok(incoming)
}

/// The filesystem half. **Every local write in a sync happens here**, on the
/// thread that owns the notebook.
///
/// `protect` is the open note when it has unsaved edits. A pull aimed at it is
/// turned into a conflict copy rather than applied, because the version in the
/// editor is one nobody has seen yet and overwriting it would lose the only
/// copy. That is the same judgement the external-change banner makes, reached
/// the same way, and it is why this cannot run on the worker: the worker does
/// not know what is open.
///
/// **The base returned is what actually happened, not what was planned.** Every
/// write that failed is left out, so the next pass sees that note as still
/// needing work — a pass that dies half way leaves the vault behind rather than
/// wrong.
///
/// Renames go first, because everything after refers to notes by their new ids.
pub fn apply(
    vault: &Vault,
    incoming: Incoming,
    protect: Option<&NoteId>,
    from: &str,
    date: &str,
) -> (Snapshot, Report) {
    let Incoming {
        land,
        rename,
        delete,
        mut agreed,
        mut report,
    } = incoming;

    for (old, new) in &rename {
        // Renaming the note being typed on would move the file out from under
        // the save tick. Left for a later pass, when it is saved.
        if protect == Some(old) {
            continue;
        }
        let hash = read_text(vault, old).map(|text| Hash::of(&text));
        match vault.rename(old, new) {
            Ok(()) => {
                agreed.remove(old);
                if let Some(hash) = hash {
                    agreed.insert(new.clone(), hash);
                }
                report.renamed += 1;
            }
            Err(_) => report.failed += 1,
        }
    }

    for landing in land {
        // The one thing the worker could not decide.
        let redirected = landing.conflict_with.is_none() && protect == Some(&landing.id);
        let (target, conflicted) = if redirected {
            (conflict_id(&landing.id, from, date), true)
        } else {
            (landing.id.clone(), landing.conflict_with.is_some())
        };

        if write_text(vault, &target, &landing.text).is_err() {
            report.failed += 1;
            continue;
        }
        if conflicted {
            // The copy is a note of its own. The base for the original stays
            // where it was, so the next pass sees both sides changed and plans
            // the push that settles it.
            report.conflicted += 1;
        } else {
            agreed.insert(target, Hash::of(&landing.text));
            report.pulled += 1;
        }
    }

    for id in &delete {
        // Deleting the note being typed on is the one deletion worth refusing
        // outright: the editor holds a version nobody else has, and the
        // vanished-note banner already covers saying so.
        if protect == Some(id) {
            continue;
        }
        match vault.delete(id) {
            Ok(()) => {
                agreed.remove(id);
                report.deleted_here += 1;
            }
            Err(_) => report.failed += 1,
        }
    }

    (agreed, report)
}

/// Both halves in one call, with nothing protected. For tests and for
/// `examples/sync_check`; the application uses [`gather`] and [`apply`].
pub fn run(
    vault: &Vault,
    base: &Snapshot,
    remote: &dyn Remote,
    from: &str,
    date: &str,
) -> Result<(Snapshot, Report), SyncError> {
    let incoming = gather(vault, base, remote, from, date)?;
    Ok(apply(vault, incoming, None, from, date))
}

fn read_text(vault: &Vault, id: &NoteId) -> Option<String> {
    vault.read(id).ok().map(|note| note.to_text())
}

/// Write through the vault rather than to the path, so a sync gets the same
/// temporary-file-then-rename discipline every other write in Brain gets.
fn write_text(vault: &Vault, id: &NoteId, text: &str) -> Result<(), ()> {
    let note = Note::from_text(id.clone(), text);
    vault.write(&note).map_err(|_| ())
}

/// Where the agreed snapshot is kept: beside the vectors, in the vault's own
/// disposable directory.
///
/// Losing it is not fatal. An empty base makes the next pass a first pass,
/// which pushes and pulls everything and calls nothing a conflict that is not
/// a genuine one — slow, and correct.
pub fn default_base_path(vault: &std::path::Path) -> std::path::PathBuf {
    vault.join(".brain").join("sync.json")
}

/// Read the agreed snapshot, or start from nothing.
pub fn load_base(path: &std::path::Path) -> Snapshot {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Snapshot::new();
    };
    let Ok(raw) = serde_json::from_str::<BTreeMap<String, u64>>(&text) else {
        return Snapshot::new();
    };
    raw.into_iter()
        .map(|(id, hash)| (NoteId::from_relative(id), Hash(hash)))
        .collect()
}

/// Write the agreed snapshot, atomically.
pub fn save_base(base: &Snapshot, path: &std::path::Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw: BTreeMap<&str, u64> = base
        .iter()
        .map(|(id, hash)| (id.as_str(), hash.0))
        .collect();
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_string(&raw)?)?;
    std::fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(path: &str) -> NoteId {
        NoteId::from_relative(path)
    }

    fn snapshot(notes: &[(&str, &str)]) -> Snapshot {
        notes
            .iter()
            .map(|(path, text)| (id(path), Hash::of(text)))
            .collect()
    }

    fn plan_of(base: &Snapshot, local: &Snapshot, remote: &Snapshot) -> Plan {
        plan(base, local, remote, "phone", "2026-08-04")
    }

    #[test]
    fn a_quiet_vault_plans_nothing() {
        let both = snapshot(&[("A.md", "one"), ("B.md", "two")]);

        assert!(plan_of(&both, &both, &both).is_empty());
    }

    #[test]
    fn a_first_pass_pushes_what_is_here_and_pulls_what_is_there() {
        let base = Snapshot::new();
        let local = snapshot(&[("A.md", "mine")]);
        let remote = snapshot(&[("B.md", "theirs")]);

        let plan = plan_of(&base, &local, &remote);

        assert_eq!(plan.push, vec![id("A.md")]);
        assert_eq!(plan.pull, vec![id("B.md")]);
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn an_edit_on_one_side_only_moves_one_way() {
        let base = snapshot(&[("A.md", "before")]);
        let local = snapshot(&[("A.md", "after")]);

        let plan = plan_of(&base, &local, &base);
        assert_eq!(plan.push, vec![id("A.md")]);
        assert!(plan.pull.is_empty());

        // And the mirror image, which is the case that gets written backwards.
        let plan = plan_of(&base, &base, &local);
        assert_eq!(plan.pull, vec![id("A.md")]);
        assert!(plan.push.is_empty());
    }

    #[test]
    fn a_deletion_on_one_side_only_is_carried_across() {
        let base = snapshot(&[("A.md", "text")]);
        let gone = Snapshot::new();

        assert_eq!(plan_of(&base, &gone, &base).delete_remote, vec![id("A.md")]);
        assert_eq!(plan_of(&base, &base, &gone).delete_local, vec![id("A.md")]);
    }

    #[test]
    fn both_sides_reaching_the_same_text_is_agreement_not_conflict() {
        let base = snapshot(&[("A.md", "before")]);
        let same = snapshot(&[("A.md", "after")]);

        assert!(plan_of(&base, &same, &same).is_empty());
    }

    #[test]
    fn both_sides_deleting_is_agreement_too() {
        let base = snapshot(&[("A.md", "text")]);
        let gone = Snapshot::new();

        assert!(plan_of(&base, &gone, &gone).is_empty());
    }

    // ---- the two rules ----

    #[test]
    fn a_deletion_never_wins_over_an_edit() {
        let base = snapshot(&[("A.md", "before")]);
        let edited = snapshot(&[("A.md", "after")]);
        let gone = Snapshot::new();

        // Deleted here, edited there: take it back.
        let plan = plan_of(&base, &gone, &edited);
        assert_eq!(plan.pull, vec![id("A.md")]);
        assert!(plan.delete_local.is_empty());
        assert!(plan.delete_remote.is_empty());

        // Edited here, deleted there: put it back.
        let plan = plan_of(&base, &edited, &gone);
        assert_eq!(plan.push, vec![id("A.md")]);
        assert!(plan.delete_local.is_empty());
        assert!(plan.delete_remote.is_empty());
    }

    #[test]
    fn a_rename_elsewhere_is_applied_here_as_a_rename() {
        let base = snapshot(&[("Ownership.md", "the text")]);
        let renamed = snapshot(&[("Borrowing.md", "the text")]);

        let plan = plan_of(&base, &base, &renamed);

        // Not a delete and a download: the content is already right, so this
        // is one `fs::rename`.
        assert_eq!(
            plan.rename_local,
            vec![(id("Ownership.md"), id("Borrowing.md"))]
        );
        assert!(plan.pull.is_empty());
        assert!(plan.delete_local.is_empty());
    }

    #[test]
    fn a_rename_here_goes_up_under_its_new_name() {
        let base = snapshot(&[("Ownership.md", "the text")]);
        let renamed = snapshot(&[("Borrowing.md", "the text")]);

        let plan = plan_of(&base, &renamed, &base);

        assert_eq!(plan.push, vec![id("Borrowing.md")]);
        assert_eq!(plan.delete_remote, vec![id("Ownership.md")]);
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn a_rename_and_an_edit_compose_rather_than_fight() {
        // One machine renamed the note; another edited it under the old name.
        let base = snapshot(&[("Ownership.md", "before")]);
        let local = snapshot(&[("Ownership.md", "after")]);
        let remote = snapshot(&[("Borrowing.md", "before")]);

        let plan = plan_of(&base, &local, &remote);

        // The rename applies, and the edit is still pushed — both changes
        // survive, and neither is a conflict to ask about.
        assert_eq!(
            plan.rename_local,
            vec![(id("Ownership.md"), id("Borrowing.md"))]
        );
        assert!(
            plan.conflicts.is_empty(),
            "a rename beside an edit was treated as a conflict: {plan:?}"
        );
        assert!(
            plan.delete_local.is_empty(),
            "the edited note was going to be deleted: {plan:?}"
        );
    }

    #[test]
    fn a_rename_to_the_same_name_on_both_sides_is_not_a_disagreement() {
        let base = snapshot(&[("Ownership.md", "the text")]);
        let renamed = snapshot(&[("Borrowing.md", "the text")]);

        let plan = plan_of(&base, &renamed, &renamed);

        assert!(plan.rename_local.is_empty());
        assert!(plan.conflicts.is_empty());
        assert!(plan.push.is_empty(), "{plan:?}");
    }

    // ---- conflicts ----

    #[test]
    fn different_text_on_both_sides_becomes_a_note_beside_it() {
        let base = snapshot(&[("Ownership.md", "before")]);
        let local = snapshot(&[("Ownership.md", "what I typed")]);
        let remote = snapshot(&[("Ownership.md", "what they typed")]);

        let plan = plan_of(&base, &local, &remote);

        assert_eq!(
            plan.conflicts,
            vec![Conflict {
                id: id("Ownership.md"),
                copy: id("Ownership (conflict 2026-08-04 from phone).md"),
            }]
        );
        // The note being typed on stays exactly where it is.
        assert!(plan.pull.is_empty());
        assert!(plan.delete_local.is_empty());
    }

    #[test]
    fn a_conflict_copy_stays_in_the_folder_its_note_is_in() {
        let copy = conflict_id(&id("Meetings/Standup.md"), "laptop", "2026-08-04");

        assert_eq!(
            copy.as_str(),
            "Meetings/Standup (conflict 2026-08-04 from laptop).md"
        );
    }

    #[test]
    fn a_conflict_copy_cannot_answer_to_the_original_s_title() {
        let original = id("Ownership.md");
        let copy = conflict_id(&original, "phone", "2026-08-04");

        // Links resolve by title, so a copy sharing one would make every
        // `[[Ownership]]` ambiguous — which is a report, not a link.
        assert_ne!(copy.title(), original.title());
    }

    #[test]
    fn a_conflicted_note_at_the_vault_root_gets_no_leading_slash() {
        assert_eq!(
            conflict_id(&id("A.md"), "phone", "2026-08-04").as_str(),
            "A (conflict 2026-08-04 from phone).md"
        );
    }

    // ---- running a pass against a fake server ----

    use std::cell::RefCell;

    /// A server that is a map, plus a switch for refusing writes the way a
    /// real one does when somebody else got there first.
    #[derive(Default)]
    struct Server {
        held: RefCell<BTreeMap<NoteId, String>>,
        refuse_writes: bool,
    }

    impl Server {
        fn with(notes: &[(&str, &str)]) -> Self {
            Self {
                held: RefCell::new(
                    notes
                        .iter()
                        .map(|(path, text)| (id(path), (*text).to_string()))
                        .collect(),
                ),
                refuse_writes: false,
            }
        }

        fn text_of(&self, path: &str) -> Option<String> {
            self.held.borrow().get(&id(path)).cloned()
        }
    }

    impl Remote for Server {
        fn list(&self) -> Result<Snapshot, SyncError> {
            Ok(self
                .held
                .borrow()
                .iter()
                .map(|(id, text)| (id.clone(), Hash::of(text)))
                .collect())
        }

        fn get(&self, ids: &[NoteId]) -> Result<Vec<(NoteId, String)>, SyncError> {
            let held = self.held.borrow();
            Ok(ids
                .iter()
                .filter_map(|id| held.get(id).map(|text| (id.clone(), text.clone())))
                .collect())
        }

        fn put(&self, id: &NoteId, text: &str, base: Option<Hash>) -> Put {
            if self.refuse_writes {
                return Put::Failed("no".into());
            }
            let mut held = self.held.borrow_mut();
            let current = held.get(id).map(|text| Hash::of(text));
            if current != base {
                return Put::Stale(current);
            }
            held.insert(id.clone(), text.to_string());
            Put::Done(Hash::of(text))
        }

        fn delete(&self, id: &NoteId, base: Option<Hash>) -> Put {
            if self.refuse_writes {
                return Put::Failed("no".into());
            }
            let mut held = self.held.borrow_mut();
            let current = held.get(id).map(|text| Hash::of(text));
            if current.is_some() && current != base {
                return Put::Stale(current);
            }
            held.remove(id);
            Put::Done(Hash(0))
        }
    }

    fn vault(notes: &[(&str, &str)]) -> (tempfile::TempDir, Vault) {
        let directory = tempfile::tempdir().expect("temp dir");
        for (path, text) in notes {
            let full = directory.path().join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).expect("parent");
            }
            std::fs::write(full, text).expect("write");
        }
        let vault = Vault::new(directory.path().to_path_buf());
        (directory, vault)
    }

    fn pass(vault: &Vault, base: &Snapshot, server: &Server) -> (Snapshot, Report) {
        run(vault, base, server, "phone", "2026-08-04").expect("pass")
    }

    #[test]
    fn a_first_pass_moves_both_ways_and_lands_on_disk() {
        let (dir, vault) = vault(&[("Mine.md", "mine")]);
        let server = Server::with(&[("Theirs.md", "theirs")]);

        let (base, report) = pass(&vault, &Snapshot::new(), &server);

        assert_eq!(report.pushed, 1);
        assert_eq!(report.pulled, 1);
        assert_eq!(server.text_of("Mine.md").as_deref(), Some("mine"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("Theirs.md")).expect("read"),
            "theirs"
        );
        // Both sides are in the base now, so a second pass is quiet.
        assert_eq!(base.len(), 2);
        assert!(pass(&vault, &base, &server).1.is_quiet());
    }

    #[test]
    fn a_conflict_lands_beside_the_note_and_leaves_it_alone() {
        let (dir, vault) = vault(&[("Ownership.md", "what I typed")]);
        let server = Server::with(&[("Ownership.md", "what they typed")]);
        let base: Snapshot = [(id("Ownership.md"), Hash::of("before"))]
            .into_iter()
            .collect();

        let (_, report) = pass(&vault, &base, &server);

        assert_eq!(report.conflicted, 1);
        // The note being typed on is untouched.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("Ownership.md")).expect("read"),
            "what I typed"
        );
        // And theirs is a real file beside it, which is why it survives the
        // app being closed.
        assert_eq!(
            std::fs::read_to_string(
                dir.path()
                    .join("Ownership (conflict 2026-08-04 from phone).md")
            )
            .expect("read"),
            "what they typed"
        );
    }

    #[test]
    fn a_deletion_elsewhere_removes_the_file_here() {
        let (dir, vault) = vault(&[("Gone.md", "text")]);
        let server = Server::default();
        let base: Snapshot = [(id("Gone.md"), Hash::of("text"))].into_iter().collect();

        let (base, report) = pass(&vault, &base, &server);

        assert_eq!(report.deleted_here, 1);
        assert!(!dir.path().join("Gone.md").exists());
        assert!(base.is_empty());
    }

    #[test]
    fn a_failed_transfer_does_not_advance_the_base() {
        let (_dir, vault) = vault(&[("A.md", "text")]);
        let server = Server {
            refuse_writes: true,
            ..Server::default()
        };

        let (base, report) = pass(&vault, &Snapshot::new(), &server);

        assert_eq!(report.failed, 1);
        assert_eq!(report.pushed, 0);
        // Left out of the base, so the next pass still sees it as work. A pass
        // that dies half way leaves the vault behind rather than wrong.
        assert!(base.is_empty(), "{base:?}");
    }

    #[test]
    fn two_machines_converge_through_the_same_server() {
        let (here_dir, here) = vault(&[("A.md", "from here")]);
        let (there_dir, there) = vault(&[("B.md", "from there")]);
        let server = Server::default();

        let (here_base, _) = pass(&here, &Snapshot::new(), &server);
        let (there_base, _) = pass(&there, &Snapshot::new(), &server);
        // The second machine's push is news to the first.
        let (_here_base, report) = pass(&here, &here_base, &server);
        assert_eq!(report.pulled, 1);

        for directory in [here_dir.path(), there_dir.path()] {
            assert!(directory.join("A.md").exists(), "{directory:?} lost A");
            assert!(directory.join("B.md").exists(), "{directory:?} lost B");
        }
        assert_eq!(there_base.len(), 2);
    }

    // ---- what the worker cannot decide ----

    #[test]
    fn a_pull_aimed_at_the_note_being_typed_on_becomes_a_copy_instead() {
        let (dir, vault) = vault(&[("Open.md", "what I am typing")]);
        let server = Server::with(&[("Open.md", "what they wrote")]);
        // The base matches the server, so this is an ordinary pull as far as
        // the planner is concerned — nothing about it looks like a conflict.
        let base: Snapshot = [(id("Open.md"), Hash::of("what I am typing"))]
            .into_iter()
            .collect();

        let incoming = gather(&vault, &base, &server, "phone", "2026-08-04").expect("gather");
        assert_eq!(incoming.land.len(), 1);
        assert!(
            incoming.land[0].conflict_with.is_none(),
            "planned as a pull"
        );

        // The main thread knows the note is open with unsaved edits, which is
        // the one thing the worker could not know.
        let (_, report) = apply(
            &vault,
            incoming,
            Some(&id("Open.md")),
            "phone",
            "2026-08-04",
        );

        assert_eq!(report.conflicted, 1);
        assert_eq!(report.pulled, 0);
        // The editor's version is the only copy of itself. It is still there.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("Open.md")).expect("read"),
            "what I am typing"
        );
        assert!(dir
            .path()
            .join("Open (conflict 2026-08-04 from phone).md")
            .exists());
    }

    #[test]
    fn the_note_being_typed_on_is_not_deleted_underneath_the_editor() {
        let (dir, vault) = vault(&[("Open.md", "unsaved work")]);
        let server = Server::default();
        let base: Snapshot = [(id("Open.md"), Hash::of("unsaved work"))]
            .into_iter()
            .collect();

        let incoming = gather(&vault, &base, &server, "phone", "2026-08-04").expect("gather");
        assert_eq!(incoming.delete, vec![id("Open.md")]);

        let (agreed, report) = apply(
            &vault,
            incoming,
            Some(&id("Open.md")),
            "phone",
            "2026-08-04",
        );

        assert_eq!(report.deleted_here, 0);
        assert!(dir.path().join("Open.md").exists());
        // Still in the base, so a later pass — once it is saved, or closed —
        // picks the deletion up again rather than forgetting it.
        assert!(agreed.contains_key(&id("Open.md")));
    }

    #[test]
    fn the_note_being_typed_on_is_not_renamed_out_from_under_the_save_tick() {
        let (dir, vault) = vault(&[("Open.md", "text")]);
        let server = Server::with(&[("Renamed.md", "text")]);
        let base: Snapshot = [(id("Open.md"), Hash::of("text"))].into_iter().collect();

        let incoming = gather(&vault, &base, &server, "phone", "2026-08-04").expect("gather");
        assert_eq!(incoming.rename, vec![(id("Open.md"), id("Renamed.md"))]);

        let (_, report) = apply(
            &vault,
            incoming,
            Some(&id("Open.md")),
            "phone",
            "2026-08-04",
        );

        assert_eq!(report.renamed, 0);
        assert!(dir.path().join("Open.md").exists());
        assert!(!dir.path().join("Renamed.md").exists());
    }

    #[test]
    fn gathering_writes_no_local_file() {
        // The property the whole split exists for: everything `gather` does is
        // safe to do on a worker thread beside a running save tick.
        let (dir, vault) = vault(&[("Mine.md", "mine")]);
        let server = Server::with(&[("Theirs.md", "theirs"), ("Gone.md", "gone")]);
        let base: Snapshot = [(id("Gone.md"), Hash::of("gone"))].into_iter().collect();

        let before = snapshot_of(&vault);
        let incoming = gather(&vault, &base, &server, "phone", "2026-08-04").expect("gather");

        assert_eq!(snapshot_of(&vault), before, "gather touched the vault");
        assert!(!dir.path().join("Theirs.md").exists());
        // It did do the network half, though — that is the point.
        assert_eq!(incoming.report.pushed, 1);
        assert_eq!(incoming.land.len(), 1);
    }

    #[test]
    fn a_conflict_still_lands_when_a_different_note_is_open() {
        // Protecting the open note must not stop anything else happening.
        let (dir, vault) = vault(&[("A.md", "mine"), ("Open.md", "typing")]);
        let server = Server::with(&[("A.md", "theirs")]);
        let base: Snapshot = [(id("A.md"), Hash::of("before"))].into_iter().collect();

        let incoming = gather(&vault, &base, &server, "phone", "2026-08-04").expect("gather");
        let (_, report) = apply(
            &vault,
            incoming,
            Some(&id("Open.md")),
            "phone",
            "2026-08-04",
        );

        assert_eq!(report.conflicted, 1);
        assert!(dir
            .path()
            .join("A (conflict 2026-08-04 from phone).md")
            .exists());
    }

    #[test]
    fn the_base_survives_a_round_trip_through_its_file() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = default_base_path(directory.path());
        let base: Snapshot = [
            (id("A.md"), Hash::of("one")),
            (id("Meetings/B.md"), Hash::of("two")),
        ]
        .into_iter()
        .collect();

        save_base(&base, &path).expect("save");

        assert_eq!(load_base(&path), base);
    }

    #[test]
    fn a_missing_or_corrupt_base_starts_a_first_pass_rather_than_failing() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = default_base_path(directory.path());

        assert!(load_base(&path).is_empty());

        std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        std::fs::write(&path, "{ not json").expect("write");
        // Slow, and correct: everything is pushed and pulled again, and
        // nothing that is not a genuine conflict is called one.
        assert!(load_base(&path).is_empty());
    }

    // ---- the hash ----

    #[test]
    fn the_hash_notices_a_change_no_model_would() {
        // Over the file, not the stripped text: retagging a note changes
        // nothing a model reads and everything about what should sync.
        let before = Hash::of("---\ntags: [rust]\n---\n\nbody");
        let after = Hash::of("---\ntags: [rust, learning]\n---\n\nbody");

        assert_ne!(before, after);
    }

    #[test]
    fn the_same_text_hashes_the_same_way_twice() {
        assert_eq!(Hash::of("the same"), Hash::of("the same"));
        assert_ne!(Hash::of("the same"), Hash::of("the sane"));
    }

    #[test]
    fn two_notes_renamed_at_once_are_not_swapped_into_each_others_places() {
        // Identical text under two names cannot say which became which. What
        // matters is that both end up renamed and neither is lost.
        let base = snapshot(&[("A.md", "same"), ("B.md", "same")]);
        let remote = snapshot(&[("C.md", "same"), ("D.md", "same")]);

        let plan = plan_of(&base, &base, &remote);

        assert_eq!(plan.rename_local.len(), 2);
        assert!(plan.delete_local.is_empty(), "{plan:?}");
        let sources: BTreeSet<&NoteId> = plan.rename_local.iter().map(|(from, _)| from).collect();
        assert_eq!(sources.len(), 2, "one note was renamed twice: {plan:?}");
    }
}
