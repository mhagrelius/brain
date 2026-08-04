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

use crate::note::NoteId;

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
