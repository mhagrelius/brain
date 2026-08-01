//! Finding notes: by title for the quick-open palette, by text for everything
//! else.
//!
//! Both read the [`Index`], both are pure functions of it, and neither builds a
//! structure of its own. A linear scan over a personal vault is microseconds —
//! the moment that stops being true is the moment to argue about an inverted
//! index, and not before.

use std::collections::BTreeMap;

use crate::model::bm25::{self, Bm25};
use crate::model::index::Index;
use crate::model::note::NoteId;
use crate::model::semantic::Store;

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

// ---- hybrid search ----------------------------------------------------------

/// Reciprocal Rank Fusion's constant. 60 is the value from the paper the method
/// comes from, and it is deliberately not tuned: the point of RRF is that it
/// fuses two rankings without knowing anything about either one's scale, so
/// there is no calibration to get wrong when the embedding model changes.
pub const RRF_K: f32 = 60.0;

/// How similar a note must be before the semantic half will name it.
///
/// Without a floor, every query returns the vault's nearest notes however far
/// away they are — an agent asking about something the vault has nothing on
/// gets three confident, irrelevant notes and no way to tell. With it, the
/// honest answer is available: nothing.
///
/// 0.55 is measured, not guessed: against `nomic-embed-text-v1.5` with its
/// prefixes, questions the vault answers scored 0.588 to 0.695, and questions
/// about things it had never heard of — a tax rule, the Eiffel tower, a curry
/// recipe — scored 0.47 to 0.55.
///
/// **The bands touch.** A question in the same register as a note ("when is my
/// passport due for renewal" against a standup note about holding a release
/// until a deadline) reached 0.65, above everything relevant. No threshold
/// separates those, which is why [`Hit`] carries where each half ranked it
/// rather than a single blended number: a caller that needs certainty can
/// require both halves to have named a note, and one that needs recall can take
/// the semantic-only hits knowing what they are. It is also why this is a
/// constant with a comment rather than a setting — the number is a property of
/// one model, and the answer to a different model is to measure again.
pub const SEMANTIC_FLOOR: f32 = 0.55;

/// How deep each half searches before fusing. A note ranked 30th lexically and
/// 2nd semantically is exactly the result hybrid search exists to surface, so
/// the fusion has to be able to see it.
const DEPTH: usize = 8;

/// One note found by hybrid search, and why.
///
/// The ranks are carried through rather than folded away because the caller is
/// often an agent: "both halves agreed" and "only the vectors liked this" are
/// different degrees of confidence, and a caller that cannot tell them apart
/// has to treat every hit the same.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub id: NoteId,
    /// The fused score. Comparable within one result set and meaningless
    /// outside it — RRF scores are ordinal, not distances.
    pub score: f32,
    /// Where the lexical half ranked it, if it named it at all.
    pub lexical: Option<usize>,
    /// Where the semantic half ranked it.
    pub semantic: Option<usize>,
    /// The cosine similarity behind `semantic`, for a caller that wants to
    /// judge how near "near" was.
    pub similarity: Option<f32>,
    /// A line worth showing: the best one containing a query term, or the
    /// note's excerpt when the match was purely semantic and no line contains
    /// the words at all.
    pub snippet: String,
}

/// Search the vault lexically, semantically, or both, and fuse the result.
///
/// `semantic` is the store and the query's own vector; `None` means no
/// embedding model was available, and the answer degrades to BM25 alone rather
/// than failing. That degradation is the normal state on a machine with no
/// model server running, so it is a first-class path, not an error case.
pub fn hybrid(
    index: &Index,
    lexical: &Bm25,
    semantic: Option<(&Store, &[f32])>,
    query: &str,
    limit: usize,
) -> Vec<Hit> {
    let depth = limit.saturating_mul(DEPTH).max(limit);

    let keyword = lexical.search(query, depth);
    let vectors = semantic
        .map(|(store, vector)| store.nearest(vector, SEMANTIC_FLOOR, depth))
        .unwrap_or_default();

    let mut fused: BTreeMap<NoteId, Hit> = BTreeMap::new();
    let mut rank_in = |ranking: &[(NoteId, f32)], lexical_side: bool| {
        for (rank, (id, score)) in ranking.iter().enumerate() {
            let hit = fused.entry(id.clone()).or_insert_with(|| Hit {
                id: id.clone(),
                score: 0.0,
                lexical: None,
                semantic: None,
                similarity: None,
                snippet: String::new(),
            });
            hit.score += 1.0 / (RRF_K + rank as f32 + 1.0);
            if lexical_side {
                hit.lexical = Some(rank);
            } else {
                hit.semantic = Some(rank);
                hit.similarity = Some(*score);
            }
        }
    };
    rank_in(&keyword, true);
    rank_in(&vectors, false);

    let mut hits: Vec<Hit> = fused.into_values().collect();
    // Ties by id: two notes found only by the same half, at ranks that fuse to
    // the same score, must not swap between identical queries.
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    hits.truncate(limit);

    for hit in &mut hits {
        hit.snippet = snippet_for(index, &hit.id, query);
    }
    hits
}

/// The line of a note worth showing next to a hit.
///
/// The line containing the most query terms, or the note's excerpt when none of
/// them appear — which is exactly what a purely semantic match looks like, and
/// showing an empty snippet there would make the best kind of hit look like a
/// mistake.
fn snippet_for(index: &Index, id: &NoteId, query: &str) -> String {
    let terms = bm25::tokenize(query);
    let text = index.text(id);

    let best = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let words = bm25::tokenize(line);
            let hits = terms
                .iter()
                .filter(|term| words.iter().any(|word| word == *term))
                .count();
            (hits, line)
        })
        .max_by_key(|(hits, _)| *hits);

    match best {
        Some((hits, line)) if hits > 0 => line.chars().take(200).collect(),
        _ => index.excerpt(id).to_string(),
    }
}

/// Where in `snippet` to underline, for a hit found by its words.
///
/// The whole query first, then its longest single term — "borrow checker"
/// underlines the phrase when a line holds it and just "checker" when the line
/// only has that. `None` when none of the words are there at all, which is what
/// a purely semantic hit looks like: nothing is underlined rather than
/// something arbitrary.
///
/// Ranges are in characters, because that is what the widget wants and a byte
/// offset into "café" is not a place a cursor can go.
pub fn highlight_of(snippet: &str, query: &str) -> Option<(usize, usize)> {
    let lowered = snippet.to_lowercase();
    let whole = query.trim().to_lowercase();

    let mut candidates = Vec::new();
    if !whole.is_empty() {
        candidates.push(whole);
    }
    let mut terms = bm25::tokenize(query);
    terms.sort_by_key(|term| std::cmp::Reverse(term.chars().count()));
    candidates.extend(terms);

    for candidate in candidates {
        let Some(at) = lowered.find(&candidate) else {
            continue;
        };
        let start = lowered[..at].chars().count();
        return Some((start, start + candidate.chars().count()));
    }
    None
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

    // ---- hybrid ----
    //
    // The vectors here are made by hand rather than by a model: what is being
    // tested is the fusion, and a test that needs a GPU to say whether ranking
    // works is a test nobody runs. The end-to-end check against a real model
    // lives in `examples/semantic_check.rs`.

    use crate::model::semantic::{Digest, Store};

    /// A store where each note is given a vector directly.
    fn vectors(notes: &[(&str, [f32; 2])]) -> Store {
        let mut store = Store::new("test-model");
        for (path, vector) in notes {
            let id = NoteId::from_relative(path);
            store.insert(&id, Digest::of(path), vec![vector.to_vec()]);
        }
        store
    }

    fn ids(hits: &[Hit]) -> Vec<&str> {
        hits.iter().map(|hit| hit.id.title()).collect()
    }

    #[test]
    fn with_no_model_hybrid_search_is_bm25_and_still_works() {
        // The normal state on a machine with no embedding server running. It
        // has to be a degradation, not a failure.
        let index = index(&[
            ("Ownership.md", "moves are destructive"),
            ("Other.md", "nothing relevant"),
        ]);
        let lexical = Bm25::build(&index);
        let hits = hybrid(&index, &lexical, None, "destructive", 5);
        assert_eq!(ids(&hits), ["Ownership"]);
        assert_eq!(hits[0].lexical, Some(0));
        assert_eq!(hits[0].semantic, None);
    }

    #[test]
    fn a_paraphrase_is_found_when_the_words_never_appear() {
        // The reason semantic search is here at all: the note says "moves are
        // destructive" and never once says "copy semantics".
        let index = index(&[
            ("Ownership.md", "moves are destructive"),
            ("Shopping.md", "milk, bread, eggs"),
        ]);
        let lexical = Bm25::build(&index);
        let store = vectors(&[("Ownership.md", [1.0, 0.0]), ("Shopping.md", [0.0, 1.0])]);

        let query = [1.0, 0.0];
        assert!(
            lexical.search("copy semantics", 10).is_empty(),
            "the lexical half must genuinely miss this"
        );
        let hits = hybrid(
            &index,
            &lexical,
            Some((&store, &query)),
            "copy semantics",
            5,
        );
        assert_eq!(ids(&hits), ["Ownership"]);
        assert_eq!(hits[0].semantic, Some(0));
        assert_eq!(hits[0].lexical, None);
    }

    #[test]
    fn a_note_both_halves_like_outranks_one_only_half_of_them_does() {
        let index = index(&[
            ("Agreed.md", "ownership and borrowing"),
            ("Lexical.md", "ownership in passing"),
            ("Semantic.md", "an unrelated wording entirely"),
        ]);
        let lexical = Bm25::build(&index);
        let store = vectors(&[
            ("Agreed.md", [1.0, 0.0]),
            ("Lexical.md", [0.0, 1.0]),
            ("Semantic.md", [0.9, 0.1]),
        ]);

        let hits = hybrid(
            &index,
            &lexical,
            Some((&store, &[1.0, 0.0])),
            "ownership",
            5,
        );
        assert_eq!(ids(&hits)[0], "Agreed");
        assert!(hits[0].lexical.is_some() && hits[0].semantic.is_some());
        // And a note only one half named still appears — fusion adds, it does
        // not intersect.
        assert!(ids(&hits).contains(&"Semantic"));
    }

    #[test]
    fn a_query_about_nothing_in_the_vault_returns_nothing() {
        // An agent handed three confident, irrelevant notes cannot tell they
        // are irrelevant. Silence is the useful answer.
        let index = index(&[("A.md", "rust ownership"), ("B.md", "borrow checker")]);
        let lexical = Bm25::build(&index);
        let store = vectors(&[("A.md", [1.0, 0.0]), ("B.md", [1.0, 0.0])]);

        let hits = hybrid(
            &index,
            &lexical,
            Some((&store, &[0.0, 1.0])),
            "sourdough starter hydration",
            5,
        );
        assert!(hits.is_empty(), "{hits:?}");
    }

    #[test]
    fn a_hit_carries_a_line_worth_reading_however_it_was_found() {
        let index = index(&[(
            "Ownership.md",
            "# Ownership\n\nMoves are destructive.\n\nBorrows are not.\n",
        )]);
        let lexical = Bm25::build(&index);

        // Lexical: the line the words are on.
        let found = hybrid(&index, &lexical, None, "destructive", 5);
        assert_eq!(found[0].snippet, "Moves are destructive.");

        // Semantic only: no line contains the query, so the excerpt stands in
        // rather than the best hit showing an empty row.
        let store = vectors(&[("Ownership.md", [1.0, 0.0])]);
        let paraphrased = hybrid(
            &index,
            &lexical,
            Some((&store, &[1.0, 0.0])),
            "copy semantics",
            5,
        );
        assert_eq!(paraphrased[0].snippet, index.excerpt(&paraphrased[0].id));
        assert!(!paraphrased[0].snippet.is_empty());
    }

    #[test]
    fn a_highlight_covers_the_phrase_when_it_is_there_and_a_word_when_it_is_not() {
        assert_eq!(
            highlight_of("one mutable borrow", "mutable borrow"),
            Some((4, 18))
        );
        // Only one of the words is on the line: underline the longest, which is
        // the one carrying the meaning.
        assert_eq!(
            highlight_of("one mutable thing", "mutable borrow"),
            Some((4, 11))
        );
        // Nothing to underline: a hit the vectors found.
        assert_eq!(
            highlight_of("moves are destructive", "copy semantics"),
            None
        );
    }

    #[test]
    fn a_highlight_is_measured_in_characters_not_bytes() {
        let (start, end) = highlight_of("café wörld", "wörld").expect("a range");
        let underlined: String = "café wörld".chars().skip(start).take(end - start).collect();
        assert_eq!(underlined, "wörld");
    }

    #[test]
    fn the_same_query_twice_gives_the_same_order() {
        let index = index(&[("A.md", "same words"), ("B.md", "same words")]);
        let lexical = Bm25::build(&index);
        let store = vectors(&[("A.md", [1.0, 0.0]), ("B.md", [1.0, 0.0])]);
        let run = || hybrid(&index, &lexical, Some((&store, &[1.0, 0.0])), "same", 5);
        assert_eq!(run(), run());
        assert_eq!(ids(&run()), ["A", "B"]);
    }

    #[test]
    fn hybrid_respects_its_limit() {
        // Only a quarter of the vault mentions it: a word in *every* note
        // narrows nothing down and BM25 rightly ignores it, so a limit test
        // built that way would be testing the wrong thing.
        let notes: Vec<(String, String)> = (0..20)
            .map(|n| {
                let body = if n % 4 == 0 {
                    format!("ownership, filler {n}")
                } else {
                    format!("filler {n}")
                };
                (format!("N{n}.md"), body)
            })
            .collect();
        let borrowed: Vec<(&str, &str)> = notes
            .iter()
            .map(|(path, body)| (path.as_str(), body.as_str()))
            .collect();
        let index = index(&borrowed);
        let lexical = Bm25::build(&index);
        assert_eq!(hybrid(&index, &lexical, None, "ownership", 3).len(), 3);
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
