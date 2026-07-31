//! Pulling links, embeds and tags out of a note, for the index.
//!
//! This does not re-implement any syntax. It reads the scanner's output, so
//! there is exactly one definition of what a wikilink is, and anything the
//! editor styles as a link is a link to the index too — including the negative
//! cases, since a `[[Target]]` inside a code span or a fence never becomes a
//! span in the first place and therefore never becomes an edge in the graph.

use std::collections::HashMap;

use super::{parse, Parsed, Style};

/// A `[[Target]]`, `[[Target|shown]]` or `![[attachment]]` in a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiLink {
    /// What it points at, verbatim and untrimmed of case. Resolution against
    /// real notes is the index's job, not the scanner's.
    pub target: String,
    /// The text shown in place of the target, when they differ.
    pub display: Option<String>,
    /// Character offset of the opening bracket.
    pub start: usize,
    /// Character offset just past the closing bracket.
    pub end: usize,
}

/// A `#tag` written in the body of a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagRef {
    /// Without the leading hash. `#project/brain` gives `project/brain`.
    pub name: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Extracted {
    /// `[[…]]` links to other notes.
    pub links: Vec<WikiLink>,
    /// `![[…]]` references to attachments.
    pub embeds: Vec<WikiLink>,
    /// `#tags` in the body. Frontmatter tags come from `frontmatter.rs`.
    pub tags: Vec<TagRef>,
}

/// Read every link, embed and tag out of a note's source.
pub fn extract(text: &str) -> Extracted {
    extract_with(text, &parse(text))
}

/// [`extract`], reusing a scan the caller already has.
pub fn extract_with(text: &str, parsed: &Parsed) -> Extracted {
    let chars: Vec<char> = text.chars().collect();

    // A styled span's opening syntax is the marker that ends where it begins.
    // For a piped link that marker is "[[Target|", which is where the target
    // survives; for a plain one it is just "[[".
    let opener: HashMap<usize, usize> = parsed
        .markers
        .iter()
        .map(|marker| (marker.end, marker.start))
        .collect();

    let slice = |from: usize, to: usize| -> String {
        chars[from.min(chars.len())..to.min(chars.len())]
            .iter()
            .collect()
    };

    let mut extracted = Extracted::default();
    for span in &parsed.spans {
        match span.style {
            Style::WikiLink => {
                let start = opener.get(&span.start).copied().unwrap_or(span.start);
                let shown = slice(span.start, span.end);
                // "[[" alone means the span *is* the target.
                let piped = slice(start, span.start)
                    .trim_start_matches('[')
                    .trim_end_matches('|')
                    .trim()
                    .to_string();
                let (target, display) = if piped.is_empty() {
                    (shown.trim().to_string(), None)
                } else {
                    (piped, Some(shown))
                };
                if target.is_empty() {
                    continue;
                }
                extracted.links.push(WikiLink {
                    target,
                    display,
                    start,
                    end: span.end + 2,
                });
            }
            Style::Embed => {
                let start = opener.get(&span.start).copied().unwrap_or(span.start);
                let inner = slice(span.start, span.end);
                // "![[diagram.png|300]]" — the part after the pipe sizes the
                // image and is not part of the filename.
                let (target, display) = match inner.split_once('|') {
                    Some((file, rest)) => (file.trim().to_string(), Some(rest.trim().to_string())),
                    None => (inner.trim().to_string(), None),
                };
                if target.is_empty() {
                    continue;
                }
                extracted.embeds.push(WikiLink {
                    target,
                    display,
                    start,
                    end: span.end + 2,
                });
            }
            Style::Tag => {
                let name = slice(span.start, span.end)
                    .trim_start_matches('#')
                    .to_string();
                if name.is_empty() {
                    continue;
                }
                extracted.tags.push(TagRef {
                    name,
                    start: span.start,
                    end: span.end,
                });
            }
            _ => {}
        }
    }
    extracted
}

/// Rewrite every link pointing at `from` so it points at `to`.
///
/// Returns `None` when nothing matched, so a rename can skip rewriting the
/// notes it does not affect rather than rewriting every file in the vault and
/// producing a diff full of no-ops.
///
/// Display text is preserved: `[[Old|the old thing]]` becomes
/// `[[New|the old thing]]`, because the words someone chose to show are theirs
/// and have nothing to do with the filename.
pub fn rewrite_target(body: &str, from: &str, to: &str) -> Option<String> {
    let matches = |target: &str| {
        let target = target.trim();
        target.eq_ignore_ascii_case(from)
            || target.eq_ignore_ascii_case(&format!("{from}.md"))
            // A link written as a path still points here.
            || target
                .rsplit('/')
                .next()
                .is_some_and(|tail| tail.eq_ignore_ascii_case(from))
    };

    let extracted = extract(body);
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for (link, embed) in extracted
        .links
        .iter()
        .map(|link| (link, false))
        .chain(extracted.embeds.iter().map(|embed| (embed, true)))
    {
        if !matches(&link.target) {
            continue;
        }
        let prefix = if embed { "![[" } else { "[[" };
        let replacement = match &link.display {
            Some(display) => format!("{prefix}{to}|{display}]]"),
            None => format!("{prefix}{to}]]"),
        };
        edits.push((link.start, link.end, replacement));
    }
    if edits.is_empty() {
        return None;
    }

    // Splice from the end, so earlier offsets stay valid.
    edits.sort_by_key(|(start, _, _)| *start);
    let chars: Vec<char> = body.chars().collect();
    let mut out = String::with_capacity(body.len());
    let mut at = 0usize;
    for (start, end, replacement) in edits {
        out.extend(chars[at..start.min(chars.len())].iter());
        out.push_str(&replacement);
        at = end.min(chars.len());
    }
    out.extend(chars[at..].iter());
    Some(out)
}

#[cfg(test)]
mod rewrite_tests {
    use super::*;

    #[test]
    fn a_plain_link_is_repointed() {
        assert_eq!(
            rewrite_target("See [[Old]] today.", "Old", "New").as_deref(),
            Some("See [[New]] today.")
        );
    }

    #[test]
    fn display_text_belongs_to_the_person_who_wrote_it() {
        assert_eq!(
            rewrite_target("See [[Old|the old thing]].", "Old", "New").as_deref(),
            Some("See [[New|the old thing]].")
        );
    }

    #[test]
    fn several_links_on_several_lines_are_all_rewritten() {
        assert_eq!(
            rewrite_target("[[Old]] and [[Old]]\nand [[Old|x]]", "Old", "New").as_deref(),
            Some("[[New]] and [[New]]\nand [[New|x]]")
        );
    }

    #[test]
    fn a_path_form_link_still_points_here() {
        assert_eq!(
            rewrite_target("See [[Meetings/Old]].", "Old", "New").as_deref(),
            Some("See [[New]].")
        );
    }

    #[test]
    fn embeds_are_rewritten_and_keep_their_size_hint() {
        assert_eq!(
            rewrite_target("![[old.png|300]]", "old.png", "new.png").as_deref(),
            Some("![[new.png|300]]")
        );
    }

    #[test]
    fn a_note_with_nothing_to_change_is_left_entirely_alone() {
        // So a rename does not produce a diff full of no-op rewrites.
        assert_eq!(rewrite_target("See [[Other]].", "Old", "New"), None);
        assert_eq!(rewrite_target("no links here", "Old", "New"), None);
        assert_eq!(rewrite_target("`[[Old]]` in code", "Old", "New"), None);
    }

    #[test]
    fn rewriting_is_multibyte_safe() {
        assert_eq!(
            rewrite_target("🎉 See [[Café]] 🎉", "Café", "Tearoom").as_deref(),
            Some("🎉 See [[Tearoom]] 🎉")
        );
    }
}
