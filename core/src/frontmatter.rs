//! The `---` metadata block at the top of a note.
//!
//! # Why this is not a YAML parser
//!
//! Four keys are understood — `tags`, `aliases`, `created`, `updated` — and
//! **everything else is preserved verbatim**. A note written by Obsidian, a
//! static site generator, or by hand carries keys this app has no opinion
//! about, and mangling them would make Brain unsafe to point at an existing
//! vault. The supported subset is line-oriented `key: value` and `[a, b]`
//! lists, which is a page of exhaustively testable code; a real YAML dependency
//! would cost a dependency *and* tempt the format to grow.
//!
//! # The round-trip rule
//!
//! Read a note and write it back without editing it, and the bytes must be
//! identical. That is a property test in `tests`, and it is what makes the
//! vault safe to keep in git — a save must never produce a diff the user did
//! not ask for. So a line is re-rendered canonically only when its value has
//! actually changed; otherwise the original line is emitted as it was found,
//! whatever its spacing or quoting.

use chrono::NaiveDate;

/// The four keys Brain understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    Tags,
    Aliases,
    Created,
    Updated,
}

impl Key {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "tags" => Some(Self::Tags),
            "aliases" => Some(Self::Aliases),
            "created" => Some(Self::Created),
            "updated" => Some(Self::Updated),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Tags => "tags",
            Self::Aliases => "aliases",
            Self::Created => "created",
            Self::Updated => "updated",
        }
    }
}

/// One `key:` and any indented lines belonging to it, or a run of lines that
/// mean nothing to us and are carried through untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Entry {
    Known { key: Key, lines: Vec<String> },
    Other(Vec<String>),
}

/// A note's metadata block.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Frontmatter {
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
    pub created: Option<NaiveDate>,
    pub updated: Option<NaiveDate>,
    /// The block as it was read, so unchanged lines survive a save byte for
    /// byte and unknown keys survive at all.
    entries: Vec<Entry>,
    /// The values as they were read, to tell "unchanged" from "edited".
    original: Option<Box<Frontmatter>>,
    /// The delimiter lines exactly as written, which may carry trailing spaces.
    open: String,
    close: Option<String>,
    /// Whether the closing delimiter was followed by a newline. A note whose
    /// last byte is the `-` of `---` is a real state — you are looking at it
    /// while typing one — and adding a newline there would be an edit.
    close_newline: bool,
}

/// Split a note into its metadata block and its body.
///
/// The body is returned as written, starting at the first character after the
/// closing delimiter's newline. A note with no frontmatter yields `None` and
/// the whole text, which is the common case and costs one line comparison.
pub fn split(text: &str) -> (Option<Frontmatter>, &str) {
    let mut lines = text.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return (None, text);
    };
    if first.trim_end_matches('\n').trim_end() != "---" {
        return (None, text);
    }

    let open = first.trim_end_matches('\n').to_string();
    let mut raw: Vec<String> = Vec::new();
    let mut close = None;
    let mut close_newline = false;
    let mut consumed = first.len();

    for line in lines {
        consumed += line.len();
        let content = line.trim_end_matches('\n');
        if content.trim_end() == "---" {
            close = Some(content.to_string());
            close_newline = line.ends_with('\n');
            break;
        }
        raw.push(content.to_string());
    }

    // An unterminated block is not frontmatter. Treating it as one would eat
    // the entire note the moment someone types "---" on the first line.
    if close.is_none() {
        return (None, text);
    }

    let mut frontmatter = Frontmatter {
        entries: group(&raw),
        open,
        close,
        close_newline,
        ..Default::default()
    };
    frontmatter.read_values();
    frontmatter.original = Some(Box::new(Frontmatter {
        tags: frontmatter.tags.clone(),
        aliases: frontmatter.aliases.clone(),
        created: frontmatter.created,
        updated: frontmatter.updated,
        ..Default::default()
    }));

    (Some(frontmatter), &text[consumed..])
}

/// Gather lines into entries: a `key:` line owns the indented lines under it.
fn group(raw: &[String]) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut index = 0;
    while index < raw.len() {
        let line = &raw[index];
        let key = key_of(line);
        let mut lines = vec![line.clone()];
        index += 1;
        if key.is_some() || key_name(line).is_some() {
            while index < raw.len() && is_continuation(&raw[index]) {
                lines.push(raw[index].clone());
                index += 1;
            }
        }
        match key {
            Some(key) => entries.push(Entry::Known { key, lines }),
            None => entries.push(Entry::Other(lines)),
        }
    }
    entries
}

/// The key name at the start of a line, whether or not we understand it.
fn key_name(line: &str) -> Option<&str> {
    if line.starts_with(char::is_whitespace) {
        return None;
    }
    let name = line.split_once(':')?.0;
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    Some(name)
}

fn key_of(line: &str) -> Option<Key> {
    Key::parse(key_name(line)?)
}

/// An indented line, or a `- item` belonging to the key above it.
fn is_continuation(line: &str) -> bool {
    line.starts_with(char::is_whitespace) || line.trim_start().starts_with("- ")
}

impl Frontmatter {
    /// Whether there is anything worth writing a `---` block for.
    ///
    /// A known key stops counting only when the user *cleared* it. One that is
    /// unset because its value never parsed — `created: last tuesday` — is
    /// still being written back verbatim, so the block still has content and
    /// dropping it would delete the line.
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
            && self.aliases.is_empty()
            && self.created.is_none()
            && self.updated.is_none()
            && self.entries.iter().all(|entry| match entry {
                Entry::Known { key, .. } => !self.is_set(*key) && self.changed(*key),
                Entry::Other(lines) => lines.iter().all(|line| line.trim().is_empty()),
            })
    }

    fn read_values(&mut self) {
        for entry in &self.entries.clone() {
            let Entry::Known { key, lines } = entry else {
                continue;
            };
            match key {
                Key::Tags => self.tags = list_value(lines),
                Key::Aliases => self.aliases = list_value(lines),
                Key::Created => self.created = date_value(lines),
                Key::Updated => self.updated = date_value(lines),
            }
        }
    }

    fn changed(&self, key: Key) -> bool {
        let Some(original) = &self.original else {
            return true;
        };
        match key {
            Key::Tags => self.tags != original.tags,
            Key::Aliases => self.aliases != original.aliases,
            Key::Created => self.created != original.created,
            Key::Updated => self.updated != original.updated,
        }
    }

    fn is_set(&self, key: Key) -> bool {
        match key {
            Key::Tags => !self.tags.is_empty(),
            Key::Aliases => !self.aliases.is_empty(),
            Key::Created => self.created.is_some(),
            Key::Updated => self.updated.is_some(),
        }
    }

    /// The canonical single line for a key, or `None` if it has no value and
    /// should not be written at all.
    fn line_for(&self, key: Key) -> Option<String> {
        let value = match key {
            Key::Tags if !self.tags.is_empty() => format!("[{}]", self.tags.join(", ")),
            Key::Aliases if !self.aliases.is_empty() => format!("[{}]", self.aliases.join(", ")),
            Key::Created => self.created?.to_string(),
            Key::Updated => self.updated?.to_string(),
            _ => return None,
        };
        Some(format!("{}: {value}", key.name()))
    }

    /// The block as text, delimiters included, ending in a newline.
    ///
    /// Empty frontmatter renders as the empty string rather than an empty
    /// block, so clearing the last tag removes the `---` rather than leaving a
    /// pair of delimiters behind.
    pub fn render(&self) -> String {
        if self.is_empty() {
            return String::new();
        }

        let mut out = String::new();
        out.push_str(&self.open);
        out.push('\n');

        let mut written = Vec::new();
        for entry in &self.entries {
            match entry {
                Entry::Other(lines) => {
                    for line in lines {
                        out.push_str(line);
                        out.push('\n');
                    }
                }
                Entry::Known { key, lines } => {
                    written.push(*key);
                    if !self.changed(*key) {
                        // Untouched: emit exactly what was read, spacing,
                        // quoting, block-list form and all.
                        for line in lines {
                            out.push_str(line);
                            out.push('\n');
                        }
                    } else if let Some(line) = self.line_for(*key) {
                        out.push_str(&line);
                        out.push('\n');
                    }
                    // A key whose value was cleared is dropped entirely.
                }
            }
        }

        // Keys set for the first time go after what was already there.
        for key in [Key::Tags, Key::Aliases, Key::Created, Key::Updated] {
            if written.contains(&key) || !self.is_set(key) {
                continue;
            }
            if let Some(line) = self.line_for(key) {
                out.push_str(&line);
                out.push('\n');
            }
        }

        out.push_str(self.close.as_deref().unwrap_or("---"));
        if self.close_newline {
            out.push('\n');
        }
        out
    }
}

/// `[a, b]`, `a, b`, or a block of `- item` lines beneath the key.
fn list_value(lines: &[String]) -> Vec<String> {
    let inline = lines[0]
        .split_once(':')
        .map(|(_, v)| v.trim())
        .unwrap_or("");
    if !inline.is_empty() {
        let inner = inline
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .unwrap_or(inline);
        return inner
            .split(',')
            .map(clean)
            .filter(|s| !s.is_empty())
            .collect();
    }
    lines[1..]
        .iter()
        .filter_map(|line| line.trim().strip_prefix("- "))
        .map(clean)
        .filter(|s| !s.is_empty())
        .collect()
}

/// A `YYYY-MM-DD` date. Anything else leaves the value unset — and because the
/// line then counts as unchanged, it is written back exactly as it was found.
fn date_value(lines: &[String]) -> Option<NaiveDate> {
    let value = lines[0].split_once(':')?.1.trim();
    NaiveDate::parse_from_str(clean(value).as_str(), "%Y-%m-%d").ok()
}

/// Trim a scalar and drop the quotes people put round tags with spaces in.
fn clean(value: &str) -> String {
    let value = value.trim();
    let unquoted = value
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(value);
    unquoted.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
    }

    #[test]
    fn a_note_without_frontmatter_is_all_body() {
        let (frontmatter, body) = split("# Title\n\nProse.");
        assert!(frontmatter.is_none());
        assert_eq!(body, "# Title\n\nProse.");
    }

    #[test]
    fn the_four_known_keys_are_read() {
        let source = "---\ntags: [rust, learning]\naliases: [Ownership]\n\
                      created: 2026-07-30\nupdated: 2026-07-31\n---\n# Title\n";
        let (frontmatter, body) = split(source);
        let frontmatter = frontmatter.expect("frontmatter");
        assert_eq!(frontmatter.tags, ["rust", "learning"]);
        assert_eq!(frontmatter.aliases, ["Ownership"]);
        assert_eq!(frontmatter.created, date("2026-07-30"));
        assert_eq!(frontmatter.updated, date("2026-07-31"));
        assert_eq!(body, "# Title\n");
    }

    #[test]
    fn tags_are_read_in_every_form_people_write_them() {
        for source in [
            "---\ntags: [rust, learning]\n---\n",
            "---\ntags: rust, learning\n---\n",
            "---\ntags:\n  - rust\n  - learning\n---\n",
            "---\ntags: [\"rust\", 'learning']\n---\n",
        ] {
            let (frontmatter, _) = split(source);
            assert_eq!(
                frontmatter.expect("frontmatter").tags,
                ["rust", "learning"],
                "{source:?}"
            );
        }
    }

    #[test]
    fn an_unterminated_block_is_not_frontmatter() {
        // Otherwise typing "---" on line one swallows the whole note.
        let source = "---\ntags: [rust]\n\nstill writing";
        let (frontmatter, body) = split(source);
        assert!(frontmatter.is_none());
        assert_eq!(body, source);
    }

    #[test]
    fn a_rule_further_down_is_not_frontmatter() {
        let source = "Prose.\n\n---\n\nMore.";
        assert!(split(source).0.is_none());
    }

    // ---- the round-trip rule ----

    #[test]
    fn reading_and_writing_without_editing_changes_no_bytes() {
        for source in [
            "---\ntags: [rust]\n---\n# Title\n",
            "---\ntags:\n  - rust\n  - learning\naliases: [A]\n---\nbody\n",
            "---\ntags:   [ rust ,learning ]\n---\n",
            "---\ncreated: 2026-07-30\n---\n",
            "---   \ntags: [a]\n---   \n",
            "---\n\ntags: [a]\n\n---\n",
        ] {
            let (frontmatter, body) = split(source);
            let frontmatter = frontmatter.expect("frontmatter");
            let round_tripped = format!("{}{body}", frontmatter.render());
            assert_eq!(round_tripped, source, "round trip changed {source:?}");
        }
    }

    #[test]
    fn unknown_keys_survive_a_save_untouched() {
        // A vault shared with another tool is full of keys we have no opinion
        // about. Losing them would make Brain unsafe to point at one.
        let source = "---\npublish: true\ncssclass: wide\ntags: [rust]\n\
                      banner: \"img.png\"\n---\nbody\n";
        let (frontmatter, body) = split(source);
        let mut frontmatter = frontmatter.expect("frontmatter");
        frontmatter.tags = vec!["rust".into(), "notes".into()];

        let written = format!("{}{body}", frontmatter.render());
        assert!(written.contains("publish: true"));
        assert!(written.contains("cssclass: wide"));
        assert!(written.contains("banner: \"img.png\""));
        assert!(written.contains("tags: [rust, notes]"));
    }

    #[test]
    fn an_unparseable_known_value_is_left_exactly_as_written() {
        // "created: last tuesday" is not a date. Rewriting it as one would be
        // inventing data; dropping it would be losing it.
        let source = "---\ncreated: last tuesday\n---\nbody\n";
        let (frontmatter, body) = split(source);
        let frontmatter = frontmatter.expect("frontmatter");
        assert_eq!(frontmatter.created, None);
        assert_eq!(format!("{}{body}", frontmatter.render()), source);
    }

    #[test]
    fn only_the_edited_key_is_rewritten() {
        let source = "---\ntags:\n  - rust\naliases:   [ Ownership ]\n---\nbody\n";
        let (frontmatter, body) = split(source);
        let mut frontmatter = frontmatter.expect("frontmatter");
        frontmatter.tags.push("learning".into());

        let written = format!("{}{body}", frontmatter.render());
        assert!(written.contains("tags: [rust, learning]"), "{written:?}");
        // Untouched, so its odd spacing and inline form survive.
        assert!(written.contains("aliases:   [ Ownership ]"), "{written:?}");
    }

    #[test]
    fn a_key_set_for_the_first_time_is_appended() {
        let source = "---\ntags: [rust]\n---\nbody\n";
        let (frontmatter, body) = split(source);
        let mut frontmatter = frontmatter.expect("frontmatter");
        frontmatter.updated = date("2026-07-31");

        assert_eq!(
            format!("{}{body}", frontmatter.render()),
            "---\ntags: [rust]\nupdated: 2026-07-31\n---\nbody\n"
        );
    }

    #[test]
    fn clearing_the_last_value_removes_the_block_entirely() {
        // Leaving an empty "---\n---" behind would be litter in every note.
        let source = "---\ntags: [rust]\n---\nbody\n";
        let (frontmatter, _) = split(source);
        let mut frontmatter = frontmatter.expect("frontmatter");
        frontmatter.tags.clear();
        assert_eq!(frontmatter.render(), "");
    }

    #[test]
    fn clearing_one_key_of_several_drops_just_that_line() {
        let source = "---\ntags: [rust]\naliases: [A]\n---\n";
        let (frontmatter, _) = split(source);
        let mut frontmatter = frontmatter.expect("frontmatter");
        frontmatter.aliases.clear();
        assert_eq!(frontmatter.render(), "---\ntags: [rust]\n---\n");
    }

    #[test]
    fn frontmatter_holding_only_unknown_keys_is_not_empty() {
        let (frontmatter, _) = split("---\npublish: true\n---\nbody\n");
        let frontmatter = frontmatter.expect("frontmatter");
        assert!(!frontmatter.is_empty());
        assert_eq!(frontmatter.render(), "---\npublish: true\n---\n");
    }

    #[test]
    fn tags_with_spaces_keep_their_quotes_stripped_but_their_words() {
        let (frontmatter, _) = split("---\ntags: [\"deep work\", rust]\n---\n");
        assert_eq!(
            frontmatter.expect("frontmatter").tags,
            ["deep work", "rust"]
        );
    }

    #[test]
    fn an_empty_block_yields_empty_values() {
        let (frontmatter, body) = split("---\n---\nbody\n");
        let frontmatter = frontmatter.expect("frontmatter");
        assert!(frontmatter.tags.is_empty());
        assert!(frontmatter.is_empty());
        assert_eq!(body, "body\n");
    }

    #[test]
    fn splitting_never_panics_on_partial_input() {
        // Every prefix of a note gets typed at some point.
        let source = "---\ntags: [rust]\ncreated: 2026-07-30\n---\n# Title\n";
        for length in 0..=source.len() {
            let prefix = &source[..length];
            let (frontmatter, body) = split(prefix);
            if let Some(frontmatter) = frontmatter {
                assert_eq!(format!("{}{body}", frontmatter.render()), prefix);
            }
        }
    }
}
