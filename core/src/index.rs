//! What the vault means, derived from what the vault contains.
//!
//! Titles, aliases, tags, the link graph and the text to search. All of it is
//! derived: the files are canonical and this can be thrown away and rebuilt at
//! any time, which is exactly what happens when the disk and the cache
//! disagree.
//!
//! # Why the graph is rebuilt rather than patched
//!
//! Creating a note changes how *other* notes' links resolve — every
//! `[[Borrow checker]]` that was dangling a moment ago now points somewhere.
//! Incrementally invalidating that is a bug farm, and the alternative is a
//! linear pass over every link in the vault, which for a personal notebook is
//! microseconds. So [`Index::rebuild_graph`] runs after any change, and the
//! per-note work — scanning a body for links and tags — is the part that is
//! done incrementally, because that is the part that is actually expensive.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::markdown;
use crate::note::{Note, NoteId};

/// What a `[[link]]` points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Exactly one note answers to this name.
    Note(NoteId),
    /// Several do. Reported rather than guessed at: picking one silently means
    /// a link that quietly points at the wrong note forever.
    Ambiguous(Vec<NoteId>),
    /// Nothing does — a link to a note that has not been written yet.
    Missing,
}

/// A link pointing *at* a note, and the line it was written on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backlink {
    pub from: NoteId,
    /// The line holding the link, with its markup stripped, so the panel can
    /// be read without opening anything.
    pub context: String,
}

/// Everything derived from one note.
#[derive(Debug, Clone, Default)]
struct Entry {
    title: String,
    aliases: Vec<String>,
    tags: Vec<String>,
    links: Vec<markdown::WikiLink>,
    embeds: Vec<markdown::WikiLink>,
    /// The body as written, which link offsets refer to.
    body: String,
    /// The body with markup stripped, which is what full-text search reads.
    text: String,
    excerpt: String,
}

#[derive(Debug, Clone, Default)]
pub struct Index {
    entries: BTreeMap<NoteId, Entry>,
    /// Lowercased title or alias to the notes answering to it.
    names: HashMap<String, Vec<NoteId>>,
    /// Lowercased tag to the notes carrying it.
    tags: BTreeMap<String, Vec<NoteId>>,
    backlinks: HashMap<NoteId, Vec<Backlink>>,
    /// Links that resolve to nothing, so the UI can offer to create them.
    missing: BTreeMap<String, Vec<NoteId>>,
}

impl Index {
    pub fn build(notes: &[Note]) -> Self {
        let mut index = Self::default();
        for note in notes {
            index.entries.insert(note.id.clone(), Entry::of(note));
        }
        index.rebuild_graph();
        index
    }

    /// Add or replace one note.
    pub fn update(&mut self, note: &Note) {
        self.entries.insert(note.id.clone(), Entry::of(note));
        self.rebuild_graph();
    }

    pub fn remove(&mut self, id: &NoteId) {
        self.entries.remove(id);
        self.rebuild_graph();
    }

    /// Move a note's derived data to a new id, for a rename.
    pub fn rename(&mut self, from: &NoteId, to: &NoteId) {
        if let Some(mut entry) = self.entries.remove(from) {
            entry.title = to.title().to_string();
            self.entries.insert(to.clone(), entry);
        }
        self.rebuild_graph();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn ids(&self) -> impl Iterator<Item = &NoteId> {
        self.entries.keys()
    }

    pub fn contains(&self, id: &NoteId) -> bool {
        self.entries.contains_key(id)
    }

    pub fn excerpt(&self, id: &NoteId) -> &str {
        self.entries
            .get(id)
            .map(|e| e.excerpt.as_str())
            .unwrap_or("")
    }

    pub(crate) fn text(&self, id: &NoteId) -> &str {
        self.entries.get(id).map(|e| e.text.as_str()).unwrap_or("")
    }

    pub fn tags_of(&self, id: &NoteId) -> &[String] {
        self.entries
            .get(id)
            .map(|e| e.tags.as_slice())
            .unwrap_or(&[])
    }

    /// What `target` points at, from a link written in `from`.
    ///
    /// Tried in order: an exact vault path, then a title, then an alias. The
    /// path form is first so `[[Meetings/Standup]]` disambiguates two notes
    /// that share a title, which is the only reason to write it that way.
    pub fn resolve(&self, target: &str, from: Option<&NoteId>) -> Resolution {
        let target = target.trim();
        if target.is_empty() {
            return Resolution::Missing;
        }

        let as_path = NoteId::from_relative(if target.ends_with(".md") {
            target.to_string()
        } else {
            format!("{target}.md")
        });
        if self.entries.contains_key(&as_path) {
            return Resolution::Note(as_path);
        }

        let Some(candidates) = self.names.get(&target.to_lowercase()) else {
            return Resolution::Missing;
        };
        match candidates.len() {
            0 => Resolution::Missing,
            1 => Resolution::Note(candidates[0].clone()),
            _ => {
                // A note linking to its own title means itself, whatever else
                // shares the name.
                if let Some(from) = from {
                    if candidates.contains(from) {
                        return Resolution::Note(from.clone());
                    }
                }
                Resolution::Ambiguous(candidates.clone())
            }
        }
    }

    /// Every note linking to this one.
    pub fn backlinks(&self, id: &NoteId) -> &[Backlink] {
        self.backlinks.get(id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Link targets that resolve to nothing, with the notes that want them.
    pub fn missing(&self) -> &BTreeMap<String, Vec<NoteId>> {
        &self.missing
    }

    /// Every tag in the vault, lowercased, in order, with its note count.
    ///
    /// Nested tags contribute to their parents: one note tagged
    /// `#project/brain` puts `project` in the list too, because a tag tree that
    /// cannot be collapsed at the top is not a tree.
    pub fn tags(&self) -> Vec<(String, usize)> {
        let mut counts: BTreeMap<String, BTreeSet<&NoteId>> = BTreeMap::new();
        for (tag, ids) in &self.tags {
            for ancestor in ancestors(tag) {
                counts.entry(ancestor).or_default().extend(ids.iter());
            }
        }
        counts
            .into_iter()
            .map(|(tag, ids)| (tag, ids.len()))
            .collect()
    }

    /// The notes carrying `tag` or any tag nested beneath it.
    pub fn notes_tagged(&self, tag: &str) -> Vec<NoteId> {
        let wanted = tag.trim_start_matches('#').to_lowercase();
        let mut ids: BTreeSet<&NoteId> = BTreeSet::new();
        for (candidate, tagged) in &self.tags {
            if candidate == &wanted || candidate.starts_with(&format!("{wanted}/")) {
                ids.extend(tagged.iter());
            }
        }
        ids.into_iter().cloned().collect()
    }

    /// Attachment filenames referenced by at least one note.
    pub fn referenced_attachments(&self) -> BTreeSet<String> {
        self.entries
            .values()
            .flat_map(|entry| entry.embeds.iter().map(|embed| embed.target.clone()))
            .collect()
    }

    /// Recompute names, tags and the link graph from the per-note entries.
    fn rebuild_graph(&mut self) {
        self.names.clear();
        self.tags.clear();
        self.backlinks.clear();
        self.missing.clear();

        for (id, entry) in &self.entries {
            let mut names = vec![entry.title.to_lowercase()];
            names.extend(entry.aliases.iter().map(|alias| alias.to_lowercase()));
            for name in names {
                let ids = self.names.entry(name).or_default();
                if !ids.contains(id) {
                    ids.push(id.clone());
                }
            }
            for tag in &entry.tags {
                self.tags
                    .entry(tag.to_lowercase())
                    .or_default()
                    .push(id.clone());
            }
        }

        // Resolution needs `names` complete, so linking is a second pass.
        let mut backlinks: HashMap<NoteId, Vec<Backlink>> = HashMap::new();
        let mut missing: BTreeMap<String, Vec<NoteId>> = BTreeMap::new();
        for (id, entry) in &self.entries {
            for link in &entry.links {
                match self.resolve(&link.target, Some(id)) {
                    Resolution::Note(target) => {
                        // A note linking to itself is not a backlink; it would
                        // fill the panel with the note you are already reading.
                        if &target == id {
                            continue;
                        }
                        let backlink = Backlink {
                            from: id.clone(),
                            context: entry.context_at(link.start),
                        };
                        backlinks.entry(target).or_default().push(backlink);
                    }
                    Resolution::Ambiguous(_) => {}
                    Resolution::Missing => missing
                        .entry(link.target.clone())
                        .or_default()
                        .push(id.clone()),
                }
            }
        }
        self.backlinks = backlinks;
        self.missing = missing;
    }
}

impl Entry {
    /// Everything derived from a note, from a single scan of it.
    ///
    /// Scanning once matters: this runs for every note in the vault at
    /// startup, and the obvious spelling — calling `note.tags()`, then
    /// `note.extracted()`, then `strip`, then `excerpt` — parses the same note
    /// four times over and made opening a vault four times slower than
    /// reading it off disk.
    fn of(note: &Note) -> Self {
        let parsed = markdown::parse(&note.body);
        let extracted = markdown::extract_with(&note.body, &parsed);
        let text = markdown::strip_with(&note.body, &parsed);

        let mut tags: Vec<String> = note
            .frontmatter
            .as_ref()
            .map(|frontmatter| frontmatter.tags.clone())
            .unwrap_or_default();
        for tag in &extracted.tags {
            if !tags
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&tag.name))
            {
                tags.push(tag.name.clone());
            }
        }

        Self {
            title: note.title().to_string(),
            aliases: note.aliases().to_vec(),
            tags,
            links: extracted.links,
            embeds: extracted.embeds,
            body: note.body.clone(),
            excerpt: excerpt_of(&text, note.title(), 120),
            text,
        }
    }

    /// The line holding the character at `offset` in the body, with its markup
    /// stripped.
    ///
    /// Stripped, because a backlink panel reading `see [[Rust|it]] soon` is
    /// harder to scan than `see it soon`, and the panel exists to be read. The
    /// line is located in the *raw* body, since that is what the link offsets
    /// refer to, and stripped afterwards.
    fn context_at(&self, offset: usize) -> String {
        // `offset` counts characters; taking that many gives a prefix whose
        // byte length is a valid index into the body, since it is a prefix.
        let prefix: String = self.body.chars().take(offset).collect();
        let start = prefix.rfind('\n').map(|index| index + 1).unwrap_or(0);
        let end = self.body[start..]
            .find('\n')
            .map(|index| start + index)
            .unwrap_or(self.body.len());
        markdown::strip(&self.body[start..end]).trim().to_string()
    }
}

/// The first line of prose that is not just the note's own title.
///
/// The same rule as [`Note::excerpt`], over text that has already been
/// stripped.
fn excerpt_of(text: &str, title: &str, limit: usize) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.eq_ignore_ascii_case(title))
        .unwrap_or("");
    if line.chars().count() <= limit {
        return line.to_string();
    }
    let mut excerpt: String = line.chars().take(limit).collect();
    excerpt.push('…');
    excerpt
}

/// `project/brain/ui` yields `project`, `project/brain`, `project/brain/ui`.
fn ancestors(tag: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for part in tag.split('/') {
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(part);
        out.push(current.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(path: &str, body: &str) -> Note {
        Note::from_text(NoteId::from_relative(path), body)
    }

    fn id(path: &str) -> NoteId {
        NoteId::from_relative(path)
    }

    #[test]
    fn a_link_resolves_to_a_note_by_title() {
        let index = Index::build(&[
            note("Rust ownership.md", "See [[Borrow checker]]."),
            note("Borrow checker.md", ""),
        ]);
        assert_eq!(
            index.resolve("Borrow checker", None),
            Resolution::Note(id("Borrow checker.md"))
        );
    }

    #[test]
    fn resolution_ignores_case_but_not_words() {
        let index = Index::build(&[note("Rust Ownership.md", "")]);
        assert_eq!(
            index.resolve("rust ownership", None),
            Resolution::Note(id("Rust Ownership.md"))
        );
        assert_eq!(index.resolve("rustownership", None), Resolution::Missing);
    }

    #[test]
    fn an_alias_resolves_too() {
        let index = Index::build(&[note(
            "Rust ownership.md",
            "---\naliases: [Ownership, Moves]\n---\n",
        )]);
        assert_eq!(
            index.resolve("Moves", None),
            Resolution::Note(id("Rust ownership.md"))
        );
    }

    #[test]
    fn two_notes_with_one_title_are_ambiguous_not_guessed_at() {
        // Picking one silently means a link that points at the wrong note
        // forever, and nothing ever tells you.
        let index = Index::build(&[note("Work/Standup.md", ""), note("Personal/Standup.md", "")]);
        match index.resolve("Standup", None) {
            Resolution::Ambiguous(candidates) => assert_eq!(candidates.len(), 2),
            other => panic!("expected ambiguity, got {other:?}"),
        }
    }

    #[test]
    fn a_path_disambiguates_a_shared_title() {
        let index = Index::build(&[note("Work/Standup.md", ""), note("Personal/Standup.md", "")]);
        assert_eq!(
            index.resolve("Work/Standup", None),
            Resolution::Note(id("Work/Standup.md"))
        );
        // With or without the extension.
        assert_eq!(
            index.resolve("Work/Standup.md", None),
            Resolution::Note(id("Work/Standup.md"))
        );
    }

    #[test]
    fn a_link_to_nothing_is_missing_and_remembered() {
        let index = Index::build(&[note("A.md", "See [[Not written yet]].")]);
        assert_eq!(index.resolve("Not written yet", None), Resolution::Missing);
        assert_eq!(
            index.missing().get("Not written yet"),
            Some(&vec![id("A.md")])
        );
    }

    #[test]
    fn backlinks_carry_the_line_they_were_written_on() {
        let index = Index::build(&[
            note("A.md", "# A\n\nMoves are destructive, see [[B]] for why.\n"),
            note("B.md", ""),
        ]);
        let backlinks = index.backlinks(&id("B.md"));
        assert_eq!(backlinks.len(), 1);
        assert_eq!(backlinks[0].from, id("A.md"));
        assert_eq!(
            backlinks[0].context,
            "Moves are destructive, see B for why."
        );
    }

    #[test]
    fn a_note_does_not_backlink_to_itself() {
        // It would fill the panel with the note you are already reading.
        let index = Index::build(&[note("A.md", "See [[A]].")]);
        assert!(index.backlinks(&id("A.md")).is_empty());
    }

    #[test]
    fn creating_a_note_resolves_links_that_were_already_written() {
        // The reason the graph is rebuilt rather than patched.
        let mut index = Index::build(&[note("A.md", "See [[B]].")]);
        assert!(index.backlinks(&id("B.md")).is_empty());

        index.update(&note("B.md", ""));
        assert_eq!(index.backlinks(&id("B.md")).len(), 1);
        assert!(index.missing().is_empty());
    }

    #[test]
    fn deleting_a_note_turns_its_inbound_links_back_into_missing_ones() {
        let mut index = Index::build(&[note("A.md", "See [[B]]."), note("B.md", "")]);
        index.remove(&id("B.md"));
        assert_eq!(index.resolve("B", None), Resolution::Missing);
        assert!(index.missing().contains_key("B"));
    }

    #[test]
    fn renaming_moves_the_notes_derived_data_to_the_new_title() {
        let mut index = Index::build(&[note("Old.md", "body")]);
        index.rename(&id("Old.md"), &id("New.md"));
        assert_eq!(index.resolve("New", None), Resolution::Note(id("New.md")));
        assert_eq!(index.resolve("Old", None), Resolution::Missing);
    }

    #[test]
    fn tags_come_from_frontmatter_and_body_and_nest() {
        let index = Index::build(&[
            note("A.md", "---\ntags: [rust]\n---\nAbout #project/brain.\n"),
            note("B.md", "More #project/planner work.\n"),
        ]);
        assert_eq!(
            index.tags(),
            vec![
                ("project".to_string(), 2),
                ("project/brain".to_string(), 1),
                ("project/planner".to_string(), 1),
                ("rust".to_string(), 1),
            ]
        );
    }

    #[test]
    fn filtering_by_a_parent_tag_includes_its_children() {
        let index = Index::build(&[
            note("A.md", "#project/brain"),
            note("B.md", "#project/planner"),
            note("C.md", "#rust"),
        ]);
        assert_eq!(index.notes_tagged("project"), vec![id("A.md"), id("B.md")]);
        assert_eq!(index.notes_tagged("#project/brain"), vec![id("A.md")]);
        assert!(index.notes_tagged("nothing").is_empty());
    }

    #[test]
    fn embeds_are_tracked_so_orphaned_attachments_can_be_found() {
        let index = Index::build(&[note("A.md", "![[diagram.png]] and ![[notes.pdf|x]]")]);
        let referenced = index.referenced_attachments();
        assert!(referenced.contains("diagram.png"));
        assert!(referenced.contains("notes.pdf"));
    }

    #[test]
    fn an_empty_vault_answers_everything_without_panicking() {
        let index = Index::build(&[]);
        assert!(index.is_empty());
        assert_eq!(index.resolve("anything", None), Resolution::Missing);
        assert!(index.backlinks(&id("nothing.md")).is_empty());
        assert!(index.tags().is_empty());
    }
}
