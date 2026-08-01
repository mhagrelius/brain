//! Okapi BM25 over the vault: the lexical half of hybrid search.
//!
//! # Why not SQLite's FTS5
//!
//! FTS5 is the obvious answer and it is the wrong one here. It would put a
//! second copy of every note in a database file that has to be kept level with
//! the vault — the exact bookkeeping problem the rest of Brain avoids by
//! deriving everything from the files. It also links a C library into a GTK app
//! that currently needs none, and hands the tokenizer's behaviour to a
//! dependency at the moment tokenization is the thing worth controlling.
//!
//! BM25 itself is twenty lines of arithmetic over counts Brain already has in
//! memory. What FTS5 would buy over this is a posting list, which matters at a
//! million documents and not at a personal vault's few thousand — the same
//! argument [`crate::model::search`] already makes about scanning.
//!
//! # What it ranks
//!
//! Term frequency saturating (`K1`), normalised by document length (`B`), times
//! inverse document frequency. In one sentence: a note that says "ownership"
//! four times beats one that says it once, but not four times as much; a long
//! note does not win for being long; and a word every note contains counts for
//! nothing.
//!
//! The title is worth [`TITLE_WEIGHT`] occurrences in the body, so a note
//! *called* "Rust ownership" outranks one that mentions it in passing — the
//! same judgement [`crate::model::search::by_text`] makes with a sort key, made
//! here with a score so it can be fused with a semantic ranking.

use std::collections::HashMap;

use crate::model::index::Index;
use crate::model::note::NoteId;

/// Term-frequency saturation. 1.2 is the standard starting point and there is
/// no corpus here to tune it against.
const K1: f32 = 1.2;
/// How much document length is normalised out. 0.75 is likewise standard.
const B: f32 = 0.75;
/// A title word counts for this many body words. Titles are short and chosen,
/// so a match in one is a much stronger signal than a match in prose.
const TITLE_WEIGHT: u32 = 4;

/// Split text into the terms that get counted.
///
/// Unicode alphanumerics, lowercased, everything else a separator. No stemming:
/// "ownership" and "owning" stay different words. Stemming would help recall
/// and hurt precision, and the semantic half of hybrid search already covers
/// the paraphrase case far better than a suffix-stripper would — that is the
/// division of labour the whole design rests on.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        if character.is_alphanumeric() {
            current.extend(character.to_lowercase());
        } else if !current.is_empty() {
            terms.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        terms.push(current);
    }
    terms
}

/// Words that are never the reason you wanted a note.
///
/// Statistics alone cannot catch these in a small vault: in four notes, "in"
/// appears in one of them and therefore looks as specific as "sourdough". That
/// is how a query about the tax treatment of a holiday let comes back holding a
/// bread recipe — the only word they share is "in", and nothing about the
/// counts says that is different from sharing "hydration".
///
/// Excluded as *evidence*, not from the index: they still count towards a
/// note's length, and Brain still has a literal substring search
/// ([`crate::model::search::by_text`]) for the rare occasion someone means the
/// word itself. English only, which is what this vault is in; a word list is
/// the one part of a search engine that cannot be derived from the corpus it is
/// searching.
const STOPWORDS: &[&str] = &[
    "a", "about", "all", "an", "and", "any", "are", "as", "at", "be", "been", "but", "by", "can",
    "did", "do", "does", "for", "from", "had", "has", "have", "he", "her", "his", "how", "i", "if",
    "in", "is", "it", "its", "me", "my", "no", "not", "of", "on", "one", "or", "our", "out", "she",
    "so", "some", "than", "that", "the", "their", "them", "then", "there", "these", "they", "this",
    "to", "up", "was", "we", "were", "what", "when", "where", "which", "who", "why", "will",
    "with", "would", "you", "your",
];

fn is_stopword(term: &str) -> bool {
    STOPWORDS.binary_search(&term).is_ok()
}

/// A vault smaller than this is too small to judge what a common word is: in a
/// three-note vault every word is in a third of the corpus, and calling that
/// noise would leave the query with nothing to match on.
const ENOUGH_TO_JUDGE: f32 = 4.0;

/// Whether a term says anything about *which* notes are wanted.
///
/// A word in more than half the vault does not narrow it down, and matching one
/// is not a lexical hit — it is the reason a search for "the tax treatment of a
/// holiday let" comes back holding three notes about Rust. Their idf is nearly
/// zero, so they barely move the ranking, but "barely" is still enough to put a
/// note in a result set that should have been empty.
fn informative(containing: f32, total: f32) -> bool {
    total < ENOUGH_TO_JUDGE || containing / total < 0.5
}

/// A match scoring less than this fraction of the best one is noise.
///
/// BM25 scores are not comparable between queries, but they are comparable
/// within one, and the gap between a real match and an accidental one is an
/// order of magnitude: asking "why is my bread so flat and dense" against a
/// vault of four notes, the note about sourdough scored 3.1 and a note about
/// GPUs scored 0.4 — for the word "so". Both are matches; only one is a hit.
///
/// This matters more than it looks, because of what happens next: fusion ranks,
/// not scores. A 0.4 and a 3.1 are rank 1 and rank 0, a hair apart, and the
/// noise then arrives at the fusion carrying the same weight as the best
/// semantic match in the vault.
const NOISE: f32 = 0.1;

/// One note's term counts.
#[derive(Debug, Clone)]
struct Document {
    id: NoteId,
    counts: HashMap<String, u32>,
    /// Total terms, title weighting included, for length normalisation.
    length: f32,
}

/// The counts every BM25 score is computed from.
///
/// Built from an [`Index`] and thrown away when the vault changes — the same
/// stance the link graph takes. Building it is one pass over text Brain has
/// already stripped of markup.
#[derive(Debug, Clone, Default)]
pub struct Bm25 {
    documents: Vec<Document>,
    /// How many documents contain each term.
    frequencies: HashMap<String, usize>,
    average_length: f32,
}

impl Bm25 {
    pub fn build(index: &Index) -> Self {
        let mut documents = Vec::new();
        let mut frequencies: HashMap<String, usize> = HashMap::new();

        for id in index.ids() {
            let mut counts: HashMap<String, u32> = HashMap::new();
            for term in tokenize(index.text(id)) {
                *counts.entry(term).or_default() += 1;
            }
            // The path, not just the title: a note in `Meetings/` is about
            // meetings, and that is worth as much as the word appearing once.
            for term in tokenize(id.as_str()) {
                *counts.entry(term).or_default() += TITLE_WEIGHT;
            }
            let length = counts.values().sum::<u32>() as f32;
            for term in counts.keys() {
                *frequencies.entry(term.clone()).or_default() += 1;
            }
            documents.push(Document {
                id: id.clone(),
                counts,
                length,
            });
        }

        let average_length = if documents.is_empty() {
            0.0
        } else {
            documents.iter().map(|d| d.length).sum::<f32>() / documents.len() as f32
        };
        Self {
            documents,
            frequencies,
            average_length,
        }
    }

    pub fn len(&self) -> usize {
        self.documents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }

    /// Notes matching `query`, best first, with their scores.
    ///
    /// A note has to contain at least one query term to appear at all: this is
    /// the half of hybrid search that is *literal*, and a lexical hit that
    /// contains none of the words asked for is not a lexical hit.
    pub fn search(&self, query: &str, limit: usize) -> Vec<(NoteId, f32)> {
        let terms: Vec<String> = tokenize(query)
            .into_iter()
            .filter(|term| !is_stopword(term))
            .collect();
        if terms.is_empty() || self.documents.is_empty() {
            return Vec::new();
        }
        let total = self.documents.len() as f32;

        let mut scored: Vec<(NoteId, f32)> = self
            .documents
            .iter()
            .filter_map(|document| {
                let mut score = 0.0;
                for term in &terms {
                    let Some(count) = document.counts.get(term) else {
                        continue;
                    };
                    let containing = *self.frequencies.get(term).unwrap_or(&0) as f32;
                    if !informative(containing, total) {
                        continue;
                    }
                    // Robertson/Sparck Jones idf with the +0.5 smoothing, and a
                    // floor at zero: without it a term in more than half the
                    // notes scores negative and matching it makes a note worse.
                    let idf = ((total - containing + 0.5) / (containing + 0.5) + 1.0).ln();
                    let frequency = *count as f32;
                    let normalised = 1.0 - B + B * (document.length / self.average_length.max(1.0));
                    score += idf * (frequency * (K1 + 1.0)) / (frequency + K1 * normalised);
                }
                (score > 0.0).then(|| (document.id.clone(), score))
            })
            .collect();

        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });

        // Everything an order of magnitude below the best match is dropped
        // rather than ranked. See NOISE: what survives here becomes a *rank*,
        // and a rank does not remember how weak it was.
        if let Some((_, best)) = scored.first() {
            let cutoff = best * NOISE;
            scored.retain(|(_, score)| *score >= cutoff);
        }
        scored.truncate(limit);
        scored
    }
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

    fn ranked(bm25: &Bm25, query: &str) -> Vec<String> {
        bm25.search(query, 10)
            .into_iter()
            .map(|(id, _)| id.title().to_string())
            .collect()
    }

    #[test]
    fn tokenizing_splits_on_anything_that_is_not_a_letter_or_digit() {
        assert_eq!(
            tokenize("Rust's ownership — moves, borrows (2024)."),
            ["rust", "s", "ownership", "moves", "borrows", "2024"]
        );
        assert_eq!(tokenize("café Übung"), ["café", "übung"]);
        assert!(tokenize("   ---   ").is_empty());
    }

    #[test]
    fn a_note_that_says_it_more_often_ranks_higher() {
        let bm25 = Bm25::build(&index(&[
            ("Once.md", "ownership is mentioned once here"),
            ("Often.md", "ownership ownership ownership ownership"),
        ]));
        assert_eq!(ranked(&bm25, "ownership"), ["Often", "Once"]);
    }

    #[test]
    fn a_long_note_does_not_win_for_being_long() {
        // Length normalisation is the whole reason to use BM25 over a raw
        // count: without it, the vault's longest note wins every query it
        // happens to mention the term in.
        let padding = "unrelated words about other subjects ".repeat(100);
        let bm25 = Bm25::build(&index(&[
            ("Padded.md", &format!("ownership {padding}")),
            ("Focused.md", "ownership"),
        ]));
        assert_eq!(ranked(&bm25, "ownership"), ["Focused", "Padded"]);
    }

    #[test]
    fn a_word_in_every_note_counts_for_much_less_than_a_rare_one() {
        // Not a stopword — a word this vault happens to use everywhere. idf is
        // what notices, and it is why "rust" is nearly worthless in a vault of
        // Rust notes while "elision" is not.
        let bm25 = Bm25::build(&index(&[
            ("A.md", "rust ownership rules"),
            ("B.md", "rust borrow checker"),
            ("C.md", "rust lifetime elision"),
        ]));
        let common = bm25.search("rust", 10);
        let rare = bm25.search("elision", 10);
        assert_eq!(common.len(), 3, "it is in every note");
        assert!(
            rare[0].1 > common[0].1 * 2.0,
            "a rare term should be worth far more: {rare:?} vs {common:?}"
        );
    }

    #[test]
    fn a_query_of_nothing_but_function_words_asks_for_nothing() {
        let bm25 = Bm25::build(&index(&[("A.md", "the ownership rules of the compiler")]));
        assert!(bm25.search("what is it about", 10).is_empty());
    }

    #[test]
    fn a_note_named_after_the_query_beats_one_that_merely_mentions_it() {
        let bm25 = Bm25::build(&index(&[
            ("Mentions.md", "ownership comes up here in passing"),
            ("Ownership.md", "the subject of this note"),
        ]));
        assert_eq!(ranked(&bm25, "ownership")[0], "Ownership");
    }

    #[test]
    fn a_folder_name_is_part_of_what_a_note_is_about() {
        let bm25 = Bm25::build(&index(&[
            ("Meetings/Tuesday.md", "we talked about the roadmap"),
            ("Other.md", "nothing to do with it"),
        ]));
        assert_eq!(ranked(&bm25, "meetings"), ["Tuesday"]);
    }

    #[test]
    fn several_query_terms_add_up() {
        let bm25 = Bm25::build(&index(&[
            ("Both.md", "ownership and borrowing together"),
            ("One.md", "ownership alone"),
        ]));
        assert_eq!(ranked(&bm25, "ownership borrowing"), ["Both", "One"]);
    }

    #[test]
    fn a_query_made_only_of_common_words_finds_nothing() {
        // "the tax treatment of a holiday let" against a vault about Rust: the
        // only words in common are the ones every note has, and matching those
        // is how an empty answer turns into three confident wrong ones.
        let bm25 = Bm25::build(&index(&[
            ("A.md", "the ownership rules of the compiler"),
            ("B.md", "the borrow checker and the rules"),
            ("C.md", "the lifetime of a reference"),
            ("D.md", "the standard library of the language"),
        ]));
        assert!(bm25.search("the tax treatment of a holiday", 10).is_empty());
        // But a real word still finds its note.
        assert_eq!(ranked(&bm25, "borrow"), ["B"]);
    }

    #[test]
    fn a_match_an_order_of_magnitude_below_the_best_is_dropped() {
        // The note that only shares the word "so" is a match and not a hit.
        // Left in, it becomes rank 1, and rank 1 arrives at the fusion looking
        // very much like rank 0.
        let bm25 = Bm25::build(&index(&[
            (
                "Sourdough.md",
                "the dough is dense and flat so shape it cold",
            ),
            ("Gpus.md", "the card is full so the model comes down"),
            ("Rust.md", "ownership and borrowing"),
            ("Standup.md", "we agreed to hold the release"),
        ]));
        assert_eq!(
            ranked(&bm25, "why is my bread so flat and dense"),
            ["Sourdough"]
        );
    }

    #[test]
    fn a_vault_too_small_to_judge_still_matches_its_own_words() {
        // In a two-note vault every word is in half the corpus. Calling that
        // noise by frequency alone would leave a small vault unsearchable, so
        // the frequency rule stands down and the word list carries it.
        let bm25 = Bm25::build(&index(&[("A.md", "the ownership"), ("B.md", "the borrow")]));
        assert_eq!(ranked(&bm25, "the ownership"), ["A"]);
    }

    #[test]
    fn a_function_word_is_not_evidence_however_rare_it_looks() {
        // Four notes, one of which happens to be the only one saying "in".
        // Statistically that is as specific as "hydration"; it is not.
        let bm25 = Bm25::build(&index(&[
            ("Sourdough.md", "bake it in a covered pot"),
            ("Rust.md", "ownership rules"),
            ("Gpus.md", "the card is full"),
            ("Standup.md", "hold the release"),
        ]));
        assert!(bm25
            .search("the tax treatment of a holiday let in Cornwall", 10)
            .is_empty());
        assert_eq!(ranked(&bm25, "covered pot"), ["Sourdough"]);
    }

    #[test]
    fn the_word_list_is_sorted_so_it_can_be_searched() {
        // `is_stopword` binary-searches it, and an unsorted list would quietly
        // fail to find half its own entries.
        let mut sorted = STOPWORDS.to_vec();
        sorted.sort_unstable();
        assert_eq!(STOPWORDS, sorted.as_slice());
        assert!(is_stopword("the") && is_stopword("your") && !is_stopword("ownership"));
    }

    #[test]
    fn a_note_containing_none_of_the_words_does_not_appear() {
        let bm25 = Bm25::build(&index(&[("A.md", "ownership"), ("B.md", "unrelated")]));
        assert_eq!(ranked(&bm25, "ownership"), ["A"]);
        assert!(bm25.search("nothing here matches", 10).is_empty());
        assert!(bm25.search("", 10).is_empty());
    }

    #[test]
    fn an_empty_vault_ranks_nothing_and_does_not_divide_by_zero() {
        let bm25 = Bm25::build(&index(&[]));
        assert!(bm25.is_empty());
        assert!(bm25.search("anything", 10).is_empty());
    }

    #[test]
    fn ties_are_broken_by_id_so_results_never_reshuffle() {
        let bm25 = Bm25::build(&index(&[("B.md", "same words"), ("A.md", "same words")]));
        assert_eq!(ranked(&bm25, "same words"), ["A", "B"]);
        assert_eq!(bm25.search("same", 10), bm25.search("same", 10));
    }

    #[test]
    fn markup_is_not_searchable_but_the_words_inside_it_are() {
        // The index stores stripped text, so this is really asserting that
        // BM25 reads that rather than the raw body.
        let bm25 = Bm25::build(&index(&[("A.md", "Moves are **destructive** here.")]));
        assert_eq!(ranked(&bm25, "destructive"), ["A"]);
    }
}
