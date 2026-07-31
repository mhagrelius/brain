//! Finding notes: by title for the quick-open palette, by text for everything
//! else.
//!
//! Both read the [`Index`], both are pure functions of it, and neither builds a
//! structure of its own. A linear scan over a personal vault is microseconds —
//! the moment that stops being true is the moment to argue about an inverted
//! index, and not before.

use crate::model::index::Index;
use crate::model::note::NoteId;

/// A note matched by name, ranked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleMatch {
    pub id: NoteId,
    pub score: i32,
    /// Which characters of the title the query matched, for highlighting.
    pub positions: Vec<usize>,
}

/// A note matched by its text, with the lines the query appears on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextMatch {
    pub id: NoteId,
    pub hits: usize,
    pub snippets: Vec<Snippet>,
}

/// One line a query appeared on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    pub text: String,
    /// Character range of the match within `text`.
    pub start: usize,
    pub end: usize,
}

/// How many lines of context to keep per note. More than this and the results
/// list becomes a document in its own right.
const SNIPPETS_PER_NOTE: usize = 3;

/// Rank notes by how well their title or path matches `query`.
///
/// A subsequence match, so "rsown" finds "Rust ownership", with the ranking
/// biased towards matches that start a word and matches that run together —
/// which is what makes typing three letters land on the note you meant.
pub fn by_title(index: &Index, query: &str, limit: usize) -> Vec<TitleMatch> {
    let query = query.trim();
    let mut matches: Vec<TitleMatch> = index
        .ids()
        .filter_map(|id| {
            if query.is_empty() {
                return Some(TitleMatch {
                    id: id.clone(),
                    score: 0,
                    positions: Vec::new(),
                });
            }
            // The title first, then the full path, so "meet/stand" works
            // without making every note in a folder match its folder's name.
            score(id.title(), query)
                // A path match is a weaker signal than a title match, so it
                // ranks below every title match rather than interleaving.
                .or_else(|| score(id.as_str(), query).map(|(score, _)| (score - 20, Vec::new())))
                .map(|(score, positions)| TitleMatch {
                    id: id.clone(),
                    score,
                    positions,
                })
        })
        .collect();

    // Highest score first; ties alphabetically so the list never reshuffles
    // between identical queries.
    matches.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
    matches.truncate(limit);
    matches
}

/// Find `query` in the text of every note.
///
/// Case-insensitive substring matching. Not word-boundary matching: people
/// search notes for fragments — half a filename, a stem, part of a URL — and a
/// word-boundary search silently fails on all of them.
pub fn by_text(index: &Index, query: &str, limit: usize) -> Vec<TextMatch> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }

    let mut matches: Vec<TextMatch> = index
        .ids()
        .filter_map(|id| {
            let text = index.text(id);
            let mut hits = 0;
            let mut snippets = Vec::new();
            for line in text.lines() {
                let lowered = line.to_lowercase();
                let mut from = 0;
                while let Some(found) = lowered[from..].find(&query) {
                    let at = from + found;
                    hits += 1;
                    if snippets.len() < SNIPPETS_PER_NOTE {
                        let start = line[..at].chars().count();
                        snippets.push(Snippet {
                            text: line.trim().to_string(),
                            // Trimming shifts the match, so the range is
                            // measured against the trimmed line the UI shows.
                            start: start.saturating_sub(leading_spaces(line)),
                            end: start.saturating_sub(leading_spaces(line)) + query.chars().count(),
                        });
                    }
                    from = at + query.len();
                }
            }
            (hits > 0).then_some(TextMatch {
                id: id.clone(),
                hits,
                snippets,
            })
        })
        .collect();

    // A title match outranks a body match: searching for "ownership" when a
    // note is called that means you want the note, not every mention of it.
    matches.sort_by(|a, b| {
        let titled = |id: &NoteId| id.title().to_lowercase().contains(&query);
        titled(&b.id)
            .cmp(&titled(&a.id))
            .then_with(|| b.hits.cmp(&a.hits))
            .then_with(|| a.id.cmp(&b.id))
    });
    matches.truncate(limit);
    matches
}

fn leading_spaces(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).count()
}

/// Subsequence score, or `None` if the query does not appear in order.
///
/// Positive contributions: a character that starts a word, a character
/// immediately after the previous match. Negative: every character skipped.
fn score(candidate: &str, query: &str) -> Option<(i32, Vec<usize>)> {
    let haystack: Vec<char> = candidate.chars().collect();
    let needles: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();

    let mut positions = Vec::with_capacity(needles.len());
    let mut score = 0;
    let mut at = 0usize;

    for (index, needle) in needles.iter().enumerate() {
        let found = haystack[at..]
            .iter()
            .position(|c| c.to_lowercase().eq(needle.to_lowercase()))?
            + at;

        if found == 0 {
            score += 15; // matching the first character is a strong signal
        } else if !haystack[found - 1].is_alphanumeric() {
            score += 10; // the start of a word
        }
        if index > 0 && positions.last() == Some(&(found - 1)) {
            score += 8; // contiguous with the previous match
        }
        score -= (found - at) as i32; // everything skipped over

        positions.push(found);
        at = found + 1;
    }

    // Shorter titles win: "Rust" beats "Rust ownership notes" for "rust".
    score -= (haystack.len() as i32 - needles.len() as i32) / 4;
    Some((score, positions))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::note::Note;

    fn index(notes: &[(&str, &str)]) -> Index {
        let notes: Vec<Note> = notes
            .iter()
            .map(|(path, body)| Note::from_text(NoteId::from_relative(path), body))
            .collect();
        Index::build(&notes)
    }

    fn titles(matches: &[TitleMatch]) -> Vec<&str> {
        matches.iter().map(|m| m.id.title()).collect()
    }

    #[test]
    fn an_exact_title_ranks_first() {
        let index = index(&[
            ("Rust ownership notes.md", ""),
            ("Rust.md", ""),
            ("Trusted sources.md", ""),
        ]);
        assert_eq!(titles(&by_title(&index, "rust", 10))[0], "Rust");
    }

    #[test]
    fn initials_find_a_multi_word_title() {
        // Typing three letters must land on the note you meant.
        let index = index(&[("Rust ownership.md", ""), ("Random other stuff.md", "")]);
        assert_eq!(titles(&by_title(&index, "ro", 10))[0], "Rust ownership");
    }

    #[test]
    fn a_subsequence_matches_but_out_of_order_does_not() {
        let index = index(&[("Rust ownership.md", "")]);
        assert_eq!(by_title(&index, "rsown", 10).len(), 1);
        assert!(by_title(&index, "ownrust", 10).is_empty());
        assert!(by_title(&index, "zzz", 10).is_empty());
    }

    #[test]
    fn a_path_matches_when_the_title_alone_does_not() {
        let index = index(&[("Meetings/Standup.md", "")]);
        assert_eq!(by_title(&index, "meet/stand", 10).len(), 1);
    }

    #[test]
    fn matched_positions_come_back_for_highlighting() {
        let index = index(&[("Rust.md", "")]);
        let matched = by_title(&index, "rt", 10);
        assert_eq!(matched[0].positions, vec![0, 3]);
    }

    #[test]
    fn an_empty_query_lists_everything_in_a_stable_order() {
        let index = index(&[("B.md", ""), ("A.md", "")]);
        assert_eq!(titles(&by_title(&index, "", 10)), ["A", "B"]);
    }

    #[test]
    fn results_are_limited_and_the_order_never_reshuffles() {
        let index = index(&[("A one.md", ""), ("A two.md", ""), ("A three.md", "")]);
        let first = by_title(&index, "a", 2);
        assert_eq!(first.len(), 2);
        assert_eq!(first, by_title(&index, "a", 2));
    }

    #[test]
    fn full_text_search_returns_the_lines_it_matched() {
        let index = index(&[(
            "Rust.md",
            "# Rust\n\nMoves are destructive.\n\nBorrowing is not.\n",
        )]);
        let matched = by_text(&index, "destructive", 10);
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].hits, 1);
        assert_eq!(matched[0].snippets[0].text, "Moves are destructive.");
        let snippet = &matched[0].snippets[0];
        assert_eq!(&snippet.text[snippet.start..snippet.end], "destructive");
    }

    #[test]
    fn full_text_search_ignores_case_and_counts_every_hit() {
        let index = index(&[("A.md", "Rust and rust and RUST.\nrust again.\n")]);
        assert_eq!(by_text(&index, "rust", 10)[0].hits, 4);
    }

    #[test]
    fn full_text_search_matches_fragments_not_only_words() {
        // People search notes for half a filename or part of a URL.
        let index = index(&[("A.md", "see example.com/some_path for more")]);
        assert_eq!(by_text(&index, "some_pa", 10).len(), 1);
    }

    #[test]
    fn full_text_search_reads_the_stripped_body_not_the_markup() {
        let index = index(&[("A.md", "Moves are **destructive** here.")]);
        // "**destructive**" would not match a search for "are destructive".
        assert_eq!(by_text(&index, "are destructive", 10).len(), 1);
    }

    #[test]
    fn a_note_named_after_the_query_outranks_one_merely_mentioning_it() {
        let index = index(&[
            ("Mentions.md", "ownership ownership ownership ownership"),
            ("Ownership.md", "a single mention of ownership"),
        ]);
        assert_eq!(by_text(&index, "ownership", 10)[0].id.title(), "Ownership");
    }

    #[test]
    fn snippets_are_capped_so_results_do_not_become_a_document() {
        let body = "needle\n".repeat(20);
        let index = index(&[("A.md", &body)]);
        let matched = by_text(&index, "needle", 10);
        assert_eq!(matched[0].hits, 20);
        assert_eq!(matched[0].snippets.len(), SNIPPETS_PER_NOTE);
    }

    #[test]
    fn an_empty_query_finds_no_text() {
        let index = index(&[("A.md", "anything")]);
        assert!(by_text(&index, "", 10).is_empty());
        assert!(by_text(&index, "   ", 10).is_empty());
    }

    #[test]
    fn searching_is_multibyte_safe() {
        let index = index(&[("Café.md", "Héllo wörld 🎉 and café")]);
        assert_eq!(by_title(&index, "café", 10).len(), 1);
        let matched = by_text(&index, "wörld", 10);
        let snippet = &matched[0].snippets[0];
        assert_eq!(
            snippet
                .text
                .chars()
                .skip(snippet.start)
                .take(snippet.end - snippet.start)
                .collect::<String>(),
            "wörld"
        );
    }
}
