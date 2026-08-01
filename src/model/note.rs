//! One note: what it is, and how its text turns into it and back.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::model::frontmatter::{self, Frontmatter};
use crate::model::markdown;

/// A note's identity: its path relative to the vault root, with `/` separators.
///
/// The file *is* the note, so the path is the identity — there is no generated
/// id to keep in sync with it, and no database row to orphan when someone moves
/// a file in Nautilus. Renaming a note therefore changes its id, which is why
/// renaming also rewrites inbound links.
///
/// Serialised as the bare path string, so the embedding cache on disk can be
/// read with `jq` and diffed by eye.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct NoteId(String);

impl NoteId {
    /// From a vault-relative path. Separators are normalised to `/` so an id
    /// means the same thing whatever wrote it.
    pub fn from_relative(path: impl AsRef<Path>) -> Self {
        let text = path
            .as_ref()
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        Self(text)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The relative path, for joining onto the vault root.
    pub fn to_path(&self) -> PathBuf {
        PathBuf::from(&self.0)
    }

    /// The filename without its `.md`, which is the note's title.
    pub fn title(&self) -> &str {
        let name = self.0.rsplit('/').next().unwrap_or(&self.0);
        name.strip_suffix(".md").unwrap_or(name)
    }

    /// The containing folder, or `None` at the vault root.
    pub fn folder(&self) -> Option<&str> {
        self.0.rsplit_once('/').map(|(folder, _)| folder)
    }
}

impl fmt::Display for NoteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A note, as read from disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: NoteId,
    /// The metadata block, if the file has one.
    pub frontmatter: Option<Frontmatter>,
    /// Everything after the metadata block, verbatim.
    pub body: String,
}

impl Note {
    /// Parse a note from a file's contents.
    pub fn from_text(id: NoteId, text: &str) -> Self {
        let (frontmatter, body) = frontmatter::split(text);
        Self {
            id,
            frontmatter,
            body: body.to_string(),
        }
    }

    /// An empty note at `id`.
    pub fn new(id: NoteId) -> Self {
        Self {
            id,
            frontmatter: None,
            body: String::new(),
        }
    }

    /// The file's contents. Round-trips byte for byte when nothing was edited.
    pub fn to_text(&self) -> String {
        match &self.frontmatter {
            Some(frontmatter) => format!("{}{}", frontmatter.render(), self.body),
            None => self.body.clone(),
        }
    }

    pub fn title(&self) -> &str {
        self.id.title()
    }

    /// Frontmatter aliases, which resolve as alternative link targets.
    pub fn aliases(&self) -> &[String] {
        self.frontmatter
            .as_ref()
            .map(|f| f.aliases.as_slice())
            .unwrap_or(&[])
    }

    /// Every tag on the note: frontmatter and body, deduplicated, in the order
    /// they are encountered. Frontmatter first, because that is where a
    /// deliberate classification goes.
    pub fn tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = self
            .frontmatter
            .as_ref()
            .map(|f| f.tags.clone())
            .unwrap_or_default();
        for tag in markdown::extract(&self.body).tags {
            if !tags
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&tag.name))
            {
                tags.push(tag.name);
            }
        }
        tags
    }

    /// Links and embeds in the body, with their character ranges.
    ///
    /// Offsets are relative to the *body*, not the file, because that is what
    /// the editor's buffer holds.
    pub fn extracted(&self) -> markdown::Extracted {
        markdown::extract(&self.body)
    }

    /// A one-line summary for search results and link pickers.
    pub fn excerpt(&self, limit: usize) -> String {
        let text = markdown::strip(&self.body);
        let title = self.title();
        let line = text
            .lines()
            .map(str::trim)
            // Most notes open with a heading repeating their own title, and a
            // row reading "Rust ownership / Rust ownership" says half as much
            // as it takes up. Skip it and show the first real line of prose.
            .find(|line| !line.is_empty() && !line.eq_ignore_ascii_case(title))
            .unwrap_or("");
        if line.chars().count() <= limit {
            return line.to_string();
        }
        let mut excerpt: String = line.chars().take(limit).collect();
        excerpt.push('…');
        excerpt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(path: &str) -> NoteId {
        NoteId::from_relative(path)
    }

    #[test]
    fn an_id_is_a_relative_path_with_forward_slashes() {
        assert_eq!(id("Meetings/standup.md").as_str(), "Meetings/standup.md");
        assert_eq!(id("Meetings/standup.md").title(), "standup");
        assert_eq!(id("Meetings/standup.md").folder(), Some("Meetings"));
        assert_eq!(id("note.md").folder(), None);
        assert_eq!(id("note.md").title(), "note");
    }

    #[test]
    fn a_title_is_the_filename_and_nothing_else() {
        // Not the first heading: a rename is then a file rename, and links
        // resolve without reading any file's contents.
        let note = Note::from_text(id("Rust ownership.md"), "# Something else\n");
        assert_eq!(note.title(), "Rust ownership");
    }

    #[test]
    fn a_note_round_trips_byte_for_byte() {
        for source in [
            "# Title\n\nProse.\n",
            "---\ntags: [rust]\n---\n\n# Title\n",
            "",
            "no trailing newline",
            "---\ntags:\n  - rust\n---\nbody",
        ] {
            let note = Note::from_text(id("n.md"), source);
            assert_eq!(note.to_text(), source, "{source:?}");
        }
    }

    #[test]
    fn tags_come_from_both_frontmatter_and_body() {
        let note = Note::from_text(
            id("n.md"),
            "---\ntags: [rust]\n---\nAbout #learning and #rust again.\n",
        );
        assert_eq!(note.tags(), ["rust", "learning"]);
    }

    #[test]
    fn tags_are_deduplicated_case_insensitively() {
        let note = Note::from_text(id("n.md"), "---\ntags: [Rust]\n---\n#rust\n");
        assert_eq!(note.tags(), ["Rust"]);
    }

    #[test]
    fn an_excerpt_is_the_first_line_of_prose_without_markup() {
        let note = Note::from_text(
            id("n.md"),
            "---\ntags: [a]\n---\n\n# Ownership\n\nMoves are **destructive**.\n",
        );
        assert_eq!(note.excerpt(80), "Ownership");

        let bodyless = Note::from_text(id("n.md"), "");
        assert_eq!(bodyless.excerpt(80), "");
    }

    #[test]
    fn an_excerpt_skips_a_heading_that_only_repeats_the_title() {
        // "Rust ownership / Rust ownership" says half as much as it takes up.
        let note = Note::from_text(
            id("Rust ownership.md"),
            "# Rust ownership\n\nMoves are destructive.\n",
        );
        assert_eq!(note.excerpt(80), "Moves are destructive.");

        // A note that is only its own title still has to say something.
        let bare = Note::from_text(id("Rust ownership.md"), "# Rust ownership\n");
        assert_eq!(bare.excerpt(80), "");

        // A heading that is not the title is real content.
        let other = Note::from_text(id("Notes.md"), "# Ownership\n\nProse.\n");
        assert_eq!(other.excerpt(80), "Ownership");
    }

    #[test]
    fn a_long_excerpt_is_cut_on_a_character_boundary() {
        let note = Note::from_text(id("n.md"), "🎉🎉🎉🎉 and more");
        assert_eq!(note.excerpt(4), "🎉🎉🎉🎉…");
    }
}
