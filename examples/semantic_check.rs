//! Hybrid search against a real embedding model.
//!
//! The unit tests prove the bookkeeping with a fake embedder: that a move costs
//! nothing, that a deleted note is forgotten, that a dead server degrades to
//! words alone. What they cannot prove is that the *vectors* are any good —
//! that a query nobody wrote the words for finds the note anyway. That needs a
//! model, so it lives here rather than in `cargo test`.
//!
//! ```sh
//! # a small embedding model, on CPU, beside whatever is on the GPU already
//! llama-server -m nomic-embed-text-v1.5.Q8_0.gguf --embeddings --pooling mean \
//!              -ngl 0 --host 127.0.0.1 --port 8081 &
//! cargo run --example semantic_check
//! ```
//!
//! It builds a vault in a temporary directory, embeds it, and then asks four
//! questions whose answers are known — including one the vault has nothing to
//! say about, which must come back empty. Every check prints its verdict and
//! the process exits non-zero if any of them failed, so this is runnable as a
//! gate rather than read as a demo.

use std::fs;

use brain::model::bm25::Bm25;
use brain::model::index::Index;
use brain::model::note::NoteId;
use brain::model::semantic::{self, Embedder};
use brain::model::{search, vault::Vault};
use brain::ui::Llama;

/// The notes the questions are asked against. Deliberately worded so the
/// obvious query for each one does *not* share its words: "how do I stop two
/// bits of code writing to the same thing" is not in the borrow checker note.
const NOTES: &[(&str, &str)] = &[
    (
        "Borrow checker.md",
        "# Borrow checker\n\nOne mutable reference, or many shared ones, never both \
         at once. The compiler proves it at build time, so a data race is a thing \
         that does not compile rather than a thing that happens on Tuesday.\n",
    ),
    (
        "Sourdough.md",
        "# Sourdough\n\nHydration is 75%. A wetter dough gives an open crumb but \
         is harder to shape. Bulk ferment until it has risen by half, then shape \
         cold and bake in a covered pot.\n",
    ),
    (
        "Local inference.md",
        "# Local inference\n\nThe 27B model fills the card, so it comes down before \
         a game and goes back up afterwards. Quantised weights trade a little \
         quality for the room to keep the context window long.\n",
    ),
    (
        "Standup 14 March.md",
        "# Standup 14 March\n\nAgreed to hold the release until the migration is \
         reversible. Ana is writing the rollback, and nobody is deploying on a \
         Friday again.\n",
    ),
];

/// A question, the note it should find, and whether the words give it away.
struct Question {
    query: &'static str,
    expected: Option<&'static str>,
    note: &'static str,
}

const QUESTIONS: &[Question] = &[
    Question {
        query: "how does rust stop two threads writing the same value",
        expected: Some("Borrow checker.md"),
        note: "a paraphrase: the note never says thread, race or writing",
    },
    Question {
        query: "why is my bread so flat and dense",
        expected: Some("Sourdough.md"),
        note: "a symptom, not a keyword",
    },
    Question {
        query: "what did we decide about shipping the migration",
        expected: Some("Standup 14 March.md"),
        note: "a question about a decision, in nobody's words",
    },
    Question {
        query: "the tax treatment of a holiday let in Cornwall",
        expected: None,
        note: "the vault has nothing on this and must say so",
    },
    Question {
        query: "how tall is the Eiffel tower",
        expected: None,
        note: "likewise, and not even in the same register",
    },
];

/// Questions the vault cannot answer but which sit close to one anyway.
///
/// Reported, not asserted. Measured against `nomic-embed-text-v1.5`, "when is
/// my passport due for renewal" scores 0.65 against a standup note about
/// holding a release until a deadline — higher than any question the vault
/// *can* answer. There is no floor that keeps this out and keeps the real
/// answers in, so pretending otherwise with a tuned constant would be a test
/// that passes by describing one vault. What the design does instead is carry
/// the provenance through: these hits are semantic-only, and a caller that
/// needs certainty can insist the lexical half named the note too.
const BORDERLINE: &[&str] = &[
    "when is my passport due for renewal",
    "quarterly VAT return deadlines",
    "my neighbour's dog keeps barking at night",
];

fn main() {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| brain::ui::DEFAULT_EMBEDDING_URL.to_string());

    let embedder = match Llama::connect(&url) {
        Ok(embedder) => embedder,
        Err(error) => {
            eprintln!("No embedding server at {url}: {error}");
            eprintln!(
                "Start one with:\n  llama-server -m <embedding model>.gguf --embeddings \
                 --pooling mean -ngl 0 --host 127.0.0.1 --port 8081"
            );
            std::process::exit(2);
        }
    };
    println!("model: {}\n", embedder.model());

    let directory = tempfile::tempdir().expect("temp dir");
    let vault = Vault::new(directory.path());
    for (path, body) in NOTES {
        vault
            .create(&NoteId::from_relative(path), body)
            .expect("create");
    }

    let index = || {
        let (notes, problems) = vault.scan();
        assert!(problems.is_empty(), "{problems:?}");
        Index::build(&notes)
    };

    // ---- embed the vault ----
    let mut store = semantic::Store::default();
    let started = std::time::Instant::now();
    let report = semantic::catch_up(&mut store, &index(), &embedder);
    println!(
        "embedded {} notes in {:?} ({} vectors held)",
        report.embedded,
        started.elapsed(),
        store.len()
    );
    if report.pending > 0 {
        eprintln!("the server stopped answering with {} to go", report.pending);
        std::process::exit(2);
    }

    let mut failures = 0;

    // ---- the questions ----
    println!("\nasking:");
    for question in QUESTIONS {
        let vector = embedder
            .embed_query(question.query)
            .expect("embed the query");
        let current = index();
        let lexical = Bm25::build(&current);
        let hits = search::hybrid(
            &current,
            &lexical,
            Some((&store, &vector)),
            question.query,
            3,
        );

        let top = hits.first().map(|hit| hit.id.as_str().to_string());
        let ok = match question.expected {
            Some(wanted) => top.as_deref() == Some(wanted),
            None => hits.is_empty(),
        };
        failures += usize::from(!ok);

        println!(
            "  [{}] {:?}\n        → {}   ({})",
            if ok { "ok" } else { "FAILED" },
            question.query,
            match &top {
                Some(id) => id.as_str(),
                None => "nothing",
            },
            question.note
        );
        for hit in &hits {
            println!(
                "          {:<22} lexical {:>5}  semantic {:>5}  similarity {:>6}",
                hit.id.as_str(),
                hit.lexical
                    .map(|rank| rank.to_string())
                    .unwrap_or_else(|| "—".into()),
                hit.semantic
                    .map(|rank| rank.to_string())
                    .unwrap_or_else(|| "—".into()),
                hit.similarity
                    .map(|score| format!("{score:.3}"))
                    .unwrap_or_else(|| "—".into()),
            );
        }
    }

    // ---- where the floor cannot help ----
    println!("\ncalibration (reported, not asserted):");
    for query in BORDERLINE {
        let vector = embedder.embed_query(query).expect("embed the query");
        let current = index();
        let hits = search::hybrid(
            &current,
            &Bm25::build(&current),
            Some((&store, &vector)),
            query,
            1,
        );
        match hits.first() {
            Some(hit) => println!(
                "  {:?}\n        → {} at {:.3}{}",
                query,
                hit.id.as_str(),
                hit.similarity.unwrap_or_default(),
                match hit.lexical {
                    Some(_) => ", and the words matched too",
                    None => ", on meaning alone — a caller wanting certainty can drop this",
                }
            ),
            None => println!("  {query:?}\n        → nothing"),
        }
    }

    // ---- and that the bookkeeping holds against a real model ----
    println!("\ncatching up:");
    let mut check = |label: &str, expected: (usize, usize, usize)| {
        let report = semantic::catch_up(&mut store, &index(), &embedder);
        let got = (report.embedded, report.moved, report.dropped);
        let ok = got == expected;
        failures += usize::from(!ok);
        println!(
            "  [{}] {label}: embedded {} moved {} dropped {}",
            if ok { "ok" } else { "FAILED" },
            report.embedded,
            report.moved,
            report.dropped
        );
    };

    check("nothing changed", (0, 0, 0));

    fs::create_dir_all(directory.path().join("Rust")).expect("mkdir");
    fs::rename(
        directory.path().join("Borrow checker.md"),
        directory.path().join("Rust/Borrow checker.md"),
    )
    .expect("mv");
    check("a note moved in a terminal", (0, 1, 0));

    fs::write(
        directory.path().join("Sourdough.md"),
        "# Sourdough\n\nHydration is 80% now, and the crumb is more open for it.\n",
    )
    .expect("edit");
    check("a note edited outside the app", (1, 0, 0));

    fs::remove_file(directory.path().join("Standup 14 March.md")).expect("rm");
    check("a note deleted outside the app", (0, 0, 1));

    // The moved note is still findable, under its new path.
    let vector = embedder
        .embed(&["how does rust stop two threads writing the same value".to_string()])
        .expect("embed")
        .remove(0);
    let current = index();
    let hits = search::hybrid(
        &current,
        &Bm25::build(&current),
        Some((&store, &vector)),
        "how does rust stop two threads writing the same value",
        3,
    );
    let found = hits.first().map(|hit| hit.id.as_str().to_string());
    let ok = found.as_deref() == Some("Rust/Borrow checker.md");
    failures += usize::from(!ok);
    println!(
        "  [{}] the moved note is found at its new path: {}",
        if ok { "ok" } else { "FAILED" },
        found.unwrap_or_else(|| "nothing".into())
    );

    println!();
    if failures > 0 {
        eprintln!("{failures} check(s) failed");
        std::process::exit(1);
    }
    println!("all checks passed");
}
