//! Vectors for the notes, and the bookkeeping that keeps them honest.
//!
//! Everything here is derived. The vault is still canonical: this store can be
//! deleted at any time and the only cost is embedding the notes again, which is
//! why it lives in the cache directory rather than anywhere near the notes.
//!
//! # Catching up is a pure function
//!
//! A note can change under Brain in four ways — edited here, edited in another
//! editor, deleted, or moved — and two of those never reach Brain's own code
//! paths at all. So nothing here listens for an event. [`plan`] is handed what
//! the vault holds now and what the store holds now, and says what to do:
//! embed these, carry these across, forget these. A rescan produces the same
//! plan whether the change came from Brain, from `mv` in a terminal, or from a
//! sync client that replaced half the vault while the app was closed.
//!
//! That is also what makes it testable without a model, a server or a display:
//! the interesting behaviour is a function from two sets of facts to a list of
//! work.
//!
//! # What is keyed on what
//!
//! The store is keyed by [`NoteId`], and each entry carries the [`Digest`] of
//! the text that was embedded. The id says *which note*, the digest says
//! *which version of it*. Both are needed:
//!
//! - same id, different digest → the note was edited, re-embed it.
//! - different id, same digest → the note was moved or renamed. The vectors
//!   are still correct, so they are carried across rather than recomputed.
//!   A move is the commonest large change in a vault — dragging a folder of
//!   fifty notes — and it is pure bookkeeping.
//! - an id the vault no longer has → forget it, whatever its digest.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::index::Index;
use crate::note::NoteId;

/// How much text goes into one vector.
///
/// The small embedding models worth running locally take 512 tokens, and a
/// chunk that overruns is truncated by the server with no complaint — the
/// second half of a long note would silently stop being searchable. ~2000
/// characters is a conservative 512 tokens for English prose.
pub const CHUNK_CHARS: usize = 2000;

/// A note is split into at most this many chunks. A 50-page note pasted into a
/// vault must not turn one save into a hundred embedding calls; past this the
/// note is long enough that the first chunks describe it well enough to find.
pub const MAX_CHUNKS: usize = 16;

/// The content fingerprint of a note, as it was embedded.
///
/// FNV-1a rather than `DefaultHasher`: this value is written to disk and
/// compared against on the next launch, and `DefaultHasher`'s output is
/// explicitly not stable between Rust releases — a toolchain upgrade would
/// silently invalidate every vector in the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Digest(pub u64);

impl Digest {
    pub fn of(text: &str) -> Self {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in text.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x1000_0000_01b3);
        }
        Self(hash)
    }
}

/// One note's vectors, and the version of the note they describe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Embedded {
    pub digest: Digest,
    /// One unit vector per chunk, in the order the chunks appear in the note.
    /// Stored normalised so similarity is a dot product and no query has to
    /// divide by a length it could have divided by once.
    pub chunks: Vec<Vec<f32>>,
}

/// What a note is currently worth embedding: its id, and the fingerprint of the
/// text that would go to the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    pub id: NoteId,
    pub digest: Digest,
}

/// A note that moved: the same content under a new id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    pub from: NoteId,
    pub to: NoteId,
}

/// The work needed to bring the store level with the vault.
///
/// Empty in every field is the normal steady state, and worth asserting on: a
/// plan that is not empty when nothing changed means something is re-embedding
/// the whole vault on a timer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// Notes with no usable vectors: new, edited, or never embedded.
    pub embed: Vec<NoteId>,
    /// Notes whose vectors exist under another id and can be carried across.
    pub moved: Vec<Moved>,
    /// Ids the vault no longer has.
    pub drop: Vec<NoteId>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.embed.is_empty() && self.moved.is_empty() && self.drop.is_empty()
    }

    /// How many calls to the model this plan costs. A move costs none, which is
    /// the whole point of noticing moves.
    pub fn embeddings(&self) -> usize {
        self.embed.len()
    }
}

/// The vectors Brain currently holds.
///
/// `model` is part of the store because vectors from two models are not
/// comparable — mixing them silently produces plausible, wrong rankings. When
/// it changes the store is emptied rather than migrated.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Store {
    /// Which model produced these vectors, as the server names it.
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    notes: BTreeMap<NoteId, Embedded>,
}

impl Store {
    pub fn new(model: &str) -> Self {
        Self {
            model: model.to_string(),
            notes: BTreeMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.notes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    pub fn contains(&self, id: &NoteId) -> bool {
        self.notes.contains_key(id)
    }

    pub fn get(&self, id: &NoteId) -> Option<&Embedded> {
        self.notes.get(id)
    }

    pub fn ids(&self) -> impl Iterator<Item = &NoteId> {
        self.notes.keys()
    }

    /// Point the store at a model, emptying it if that is a different one.
    ///
    /// Reports whether anything was thrown away, so the caller can say why the
    /// vault is being embedded again rather than appearing to stall.
    pub fn set_model(&mut self, model: &str) -> bool {
        if self.model == model {
            return false;
        }
        let had = !self.notes.is_empty();
        self.model = model.to_string();
        self.notes.clear();
        had
    }

    pub fn insert(&mut self, id: &NoteId, digest: Digest, chunks: Vec<Vec<f32>>) {
        self.notes.insert(
            id.clone(),
            Embedded {
                digest,
                chunks: chunks.into_iter().map(|chunk| normalise(&chunk)).collect(),
            },
        );
    }

    /// Carry out a plan's bookkeeping — the moves and the drops.
    ///
    /// The embeddings are left to the caller because they need a model and this
    /// does not. Applied before any embedding starts, so a store written out
    /// after a crash mid-embed is stale rather than wrong.
    pub fn apply(&mut self, plan: &Plan) {
        for moved in &plan.moved {
            if let Some(embedded) = self.notes.remove(&moved.from) {
                self.notes.insert(moved.to.clone(), embedded);
            }
        }
        for id in &plan.drop {
            self.notes.remove(id);
        }
    }

    /// The notes most like `query`, best first.
    ///
    /// A note scores as its best chunk: a query about one paragraph of a long
    /// note should find that note, and averaging the chunks would bury it under
    /// notes that are shorter but only vaguely related.
    ///
    /// `floor` drops everything below a similarity, so a query about nothing in
    /// the vault returns nothing rather than the least-unrelated note — which
    /// matters most when the caller is an agent, since a confident irrelevant
    /// answer is worse than an empty one.
    pub fn nearest(&self, query: &[f32], floor: f32, limit: usize) -> Vec<(NoteId, f32)> {
        let query = normalise(query);
        let mut scored: Vec<(NoteId, f32)> = self
            .notes
            .iter()
            .filter_map(|(id, embedded)| {
                let best = embedded
                    .chunks
                    .iter()
                    .map(|chunk| dot(&query, chunk))
                    .fold(f32::NEG_INFINITY, f32::max);
                (best >= floor).then(|| (id.clone(), best))
            })
            .collect();

        // Ties by id, so two notes that are equally close never swap places
        // between identical queries.
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        scored.truncate(limit);
        scored
    }
}

/// Where a vault's vectors are cached.
///
/// `$XDG_CACHE_HOME/brain/vectors-<digest>.json`, the digest being the vault's
/// path: two vaults have two stores, and neither is inside a vault, so the
/// promise that deleting Brain leaves nothing behind in your notes still holds.
/// The cache directory is the right place precisely because losing it is
/// survivable — it costs one pass of embedding and nothing else.
pub fn default_store_path(vault: &std::path::Path) -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".cache"))
        })
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let Digest(digest) = Digest::of(&vault.to_string_lossy());
    base.join("brain")
        .join(format!("vectors-{digest:016x}.json"))
}

impl Store {
    /// Read the store back, or start empty.
    ///
    /// A missing or unreadable cache is not an error and is not reported: this
    /// is derived data, and the recovery — embed the vault again — is the same
    /// one the first run performs anyway. Anything else would be an error
    /// dialog about a file the user has never heard of.
    pub fn load(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Write the store out atomically: tmp, flush, fsync, rename.
    ///
    /// The same discipline as a note, for a different reason — a half-written
    /// cache that parses is worse than no cache, because nothing would ever
    /// notice it was truncated.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string(self)?;
        let temporary = path.with_extension("json.tmp");
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(text.as_bytes())?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    }
}

/// What it would take to bring `store` level with `wanted`.
///
/// Pure, and total: every id in either input ends up in exactly one of the
/// plan's three lists or in none of them, and the result does not depend on the
/// order `wanted` arrives in.
pub fn plan(store: &Store, wanted: &[Wanted]) -> Plan {
    let mut plan = Plan::default();

    // Stored notes the vault no longer lists, indexed by digest so a note that
    // moved can be recognised by its content before it is dropped. Several
    // notes can share content — two copies of the same checklist — so each
    // digest keeps a queue and a claim takes from it.
    let live: std::collections::BTreeSet<&NoteId> = wanted.iter().map(|w| &w.id).collect();
    let mut orphans: BTreeMap<Digest, Vec<NoteId>> = BTreeMap::new();
    for (id, embedded) in &store.notes {
        if !live.contains(id) {
            orphans.entry(embedded.digest).or_default().push(id.clone());
        }
    }

    for want in wanted {
        match store.notes.get(&want.id) {
            // Already embedded, and the note has not changed since.
            Some(embedded) if embedded.digest == want.digest => {}
            // Same note, different content: it was edited, here or elsewhere.
            Some(_) => plan.embed.push(want.id.clone()),
            None => match orphans.get_mut(&want.digest).and_then(Vec::pop) {
                // This content is already embedded under an id the vault has
                // stopped listing. That is a move or a rename, and the vectors
                // are still correct.
                Some(from) => plan.moved.push(Moved {
                    from,
                    to: want.id.clone(),
                }),
                None => plan.embed.push(want.id.clone()),
            },
        }
    }

    // Whatever was not claimed by a move is genuinely gone.
    plan.drop = orphans.into_values().flatten().collect();
    plan.drop.sort();
    plan
}

// ---- catching up ------------------------------------------------------------

/// Where vectors come from.
///
/// A trait because the thing on the other side is a model server on the
/// network, and every interesting property of this module — that a move costs
/// nothing, that a deleted note is forgotten, that a failed server leaves the
/// store usable — has to be testable without one. The implementation that talks
/// HTTP lives in `src/ui/`, on the far side of the GTK line.
///
/// Synchronous on purpose: the caller runs it on a worker thread, and a trait
/// that returns futures would put an async runtime in the model layer to save
/// one `thread::spawn`.
pub trait Embedder {
    /// What the server calls the model, so the store can tell when it changed.
    fn model(&self) -> String;

    /// One vector per note chunk, in order. Anything else is a protocol error
    /// and the caller treats it as a failure rather than guessing at the
    /// alignment.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;

    /// Embed a *question* rather than a passage.
    ///
    /// A separate method because the retrieval models worth running locally are
    /// trained asymmetrically: nomic wants `search_document:` on the note and
    /// `search_query:` on the question, E5 wants `passage:` and `query:`. Send
    /// a question through the document path and the vectors still come back and
    /// still rank — a little worse, in a way nothing detects. Measured on a
    /// four-note vault, the prefixes moved the intended note's similarity from
    /// 0.57 to 0.62 and pulled the whole relevant band clear of 0.55.
    ///
    /// Defaulted to the passage path, so a fake in a test implements one method
    /// and a model with no such convention says nothing.
    fn embed_query(&self, query: &str) -> Result<Vec<f32>, EmbedError> {
        self.embed(std::slice::from_ref(&query.to_string()))?
            .into_iter()
            .next()
            .ok_or_else(|| EmbedError("no vector came back for the query".into()))
    }
}

/// Why embedding did not happen. One string: every caller's recovery is the
/// same — leave the note unembedded and try again next time — so a taxonomy of
/// failures would only give them something to ignore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedError(pub String);

impl std::fmt::Display for EmbedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What one catch-up pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub embedded: usize,
    pub moved: usize,
    pub dropped: usize,
    /// Notes still waiting for a model, because the server stopped answering
    /// part way through. Not an error — the next pass picks them up.
    pub pending: usize,
    /// The store was emptied because the model changed underneath it.
    pub reset: bool,
}

impl Report {
    /// Whether anything happened. A quiet vault reports nothing at all, which
    /// is what makes it safe to run this after every rescan.
    pub fn is_quiet(&self) -> bool {
        *self == Self::default()
    }
}

/// What the vault currently wants embedded.
pub fn wanted(index: &Index) -> Vec<Wanted> {
    index
        .ids()
        .map(|id| Wanted {
            id: id.clone(),
            digest: digest_of(id.title(), index.text(id)),
        })
        .collect()
}

/// Bring the store level with the vault, embedding whatever is missing.
///
/// The whole lifecycle in one call: notice what changed, carry moves across,
/// forget what is gone, embed the rest. It does not care *how* the vault came
/// to differ — a note saved in Brain, a folder moved in Nautilus, and a sync
/// client rewriting a hundred files while the app was shut are the same input.
///
/// A server that stops answering ends the pass rather than failing it: what was
/// embedded stays embedded, the rest is reported as pending, and the next pass
/// resumes. Hammering a server that just refused fifty requests with fifty more
/// is how a laptop's fans come on.
pub fn catch_up(store: &mut Store, index: &Index, embedder: &dyn Embedder) -> Report {
    let mut report = Report {
        reset: store.set_model(&embedder.model()),
        ..Report::default()
    };

    let wanted = wanted(index);
    let plan = plan(store, &wanted);
    report.moved = plan.moved.len();
    report.dropped = plan.drop.len();
    // Bookkeeping first: it needs no model, so a store written out after this
    // point is behind rather than wrong, whatever the server does next.
    store.apply(&plan);

    for id in &plan.embed {
        let text = index.text(id);
        let pieces = chunks(id.title(), text);
        if pieces.is_empty() {
            continue;
        }
        match embedder.embed(&pieces) {
            Ok(vectors) if vectors.len() == pieces.len() => {
                store.insert(id, digest_of(id.title(), text), vectors);
                report.embedded += 1;
            }
            _ => {
                report.pending = plan.embed.len() - report.embedded;
                break;
            }
        }
    }
    report
}

/// Split a note into the pieces that get embedded.
///
/// Headings first, because a heading is the author saying where one subject
/// ends; then paragraphs, when a section is longer than a chunk. A note under
/// the budget is one chunk, which is almost every note.
///
/// The title is prepended to every chunk. A chunk in the middle of a long note
/// often does not name what it is about, and "the model that runs on the 5090"
/// only means something under the note called "Local inference".
pub fn chunks(title: &str, text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();

    let flush = |current: &mut String, chunks: &mut Vec<String>| {
        let trimmed = current.trim();
        if !trimmed.is_empty() {
            chunks.push(format!("{title}\n\n{trimmed}"));
        }
        current.clear();
    };

    for block in text.split("\n\n") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }
        let heading = block.starts_with('#');
        if (heading || current.chars().count() + block.chars().count() > CHUNK_CHARS)
            && !current.is_empty()
        {
            flush(&mut current, &mut chunks);
        }
        // A single block longer than the budget is cut at the budget. Cutting
        // on a character boundary rather than a byte one, since a chunk split
        // through a multi-byte character is not text.
        if block.chars().count() > CHUNK_CHARS {
            let mut rest: Vec<char> = block.chars().collect();
            while !rest.is_empty() {
                let take = rest.len().min(CHUNK_CHARS);
                let piece: String = rest.drain(..take).collect();
                current.push_str(&piece);
                flush(&mut current, &mut chunks);
            }
            continue;
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(block);
    }
    flush(&mut current, &mut chunks);

    // A note with a title and no body still has one thing worth matching.
    if chunks.is_empty() && !title.trim().is_empty() {
        chunks.push(title.to_string());
    }
    chunks.truncate(MAX_CHUNKS);
    chunks
}

/// The fingerprint of a note as it would be embedded.
///
/// Taken over the chunks rather than the raw file: it is the text that reaches
/// the model that decides whether a vector is still valid, so an edit that
/// changes nothing about that text — reordering frontmatter, say — costs
/// nothing.
pub fn digest_of(title: &str, text: &str) -> Digest {
    Digest::of(&chunks(title, text).join("\u{1}"))
}

fn normalise(vector: &[f32]) -> Vec<f32> {
    let length = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if length == 0.0 || !length.is_finite() {
        return vector.to_vec();
    }
    vector.iter().map(|v| v / length).collect()
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        // Vectors of two different models, or a truncated store. Not comparable
        // and not an error: the note simply does not match, and the next plan
        // will re-embed it.
        return f32::NEG_INFINITY;
    }
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(path: &str) -> NoteId {
        NoteId::from_relative(path)
    }

    fn want(path: &str, text: &str) -> Wanted {
        Wanted {
            id: id(path),
            digest: Digest::of(text),
        }
    }

    /// A store holding one vector per note, so plans are readable.
    fn store(notes: &[(&str, &str)]) -> Store {
        let mut store = Store::new("test-model");
        for (path, text) in notes {
            store.insert(&id(path), Digest::of(text), vec![vec![1.0, 0.0]]);
        }
        store
    }

    fn paths(ids: &[NoteId]) -> Vec<&str> {
        ids.iter().map(NoteId::as_str).collect()
    }

    #[test]
    fn a_vault_that_has_not_changed_is_no_work_at_all() {
        let store = store(&[("A.md", "alpha"), ("B.md", "beta")]);
        let plan = plan(&store, &[want("A.md", "alpha"), want("B.md", "beta")]);
        assert!(plan.is_empty(), "{plan:?}");
    }

    #[test]
    fn a_new_note_is_embedded_and_nothing_else_is() {
        let store = store(&[("A.md", "alpha")]);
        let plan = plan(&store, &[want("A.md", "alpha"), want("New.md", "new")]);
        assert_eq!(paths(&plan.embed), ["New.md"]);
        assert!(plan.drop.is_empty());
    }

    #[test]
    fn an_edited_note_is_re_embedded() {
        // The same id with different content: the note was edited, in Brain or
        // in any other editor. The store cannot tell which, and does not care.
        let store = store(&[("A.md", "alpha")]);
        let plan = plan(&store, &[want("A.md", "alpha, rewritten")]);
        assert_eq!(paths(&plan.embed), ["A.md"]);
        assert!(plan.moved.is_empty());
    }

    #[test]
    fn a_deleted_note_is_dropped() {
        let store = store(&[("A.md", "alpha"), ("B.md", "beta")]);
        let plan = plan(&store, &[want("A.md", "alpha")]);
        assert_eq!(paths(&plan.drop), ["B.md"]);
        assert!(plan.embed.is_empty());
    }

    #[test]
    fn a_moved_note_keeps_its_vectors_and_costs_no_embedding() {
        // Dragging a folder of notes is the commonest big change a vault sees.
        // Re-embedding it would be minutes of GPU for a rename.
        let store = store(&[("Inbox/A.md", "alpha")]);
        let plan = plan(&store, &[want("Archive/A.md", "alpha")]);
        assert_eq!(
            plan.moved,
            [Moved {
                from: id("Inbox/A.md"),
                to: id("Archive/A.md"),
            }]
        );
        assert_eq!(plan.embeddings(), 0);
        assert!(plan.drop.is_empty(), "a move is not a delete");
    }

    #[test]
    fn a_note_that_moved_and_was_edited_is_re_embedded_not_carried() {
        let store = store(&[("Inbox/A.md", "alpha")]);
        let plan = plan(&store, &[want("Archive/A.md", "alpha and more")]);
        assert_eq!(paths(&plan.embed), ["Archive/A.md"]);
        assert_eq!(paths(&plan.drop), ["Inbox/A.md"]);
        assert!(plan.moved.is_empty());
    }

    #[test]
    fn two_notes_with_identical_content_do_not_steal_each_others_vectors() {
        // Same digest, two ids. Dropping one must not make the other look moved
        // and leave the vault with a vector fewer than it has notes.
        let store = store(&[("A.md", "same"), ("B.md", "same")]);
        let plan = plan(&store, &[want("A.md", "same"), want("C.md", "same")]);
        assert_eq!(
            plan.moved,
            [Moved {
                from: id("B.md"),
                to: id("C.md")
            }]
        );
        assert!(plan.drop.is_empty());
        assert!(plan.embed.is_empty());
    }

    #[test]
    fn a_whole_folder_moving_costs_nothing() {
        let store = store(&[
            ("Work/A.md", "a"),
            ("Work/B.md", "b"),
            ("Work/Deep/C.md", "c"),
        ]);
        let wanted = [
            want("Archive/Work/A.md", "a"),
            want("Archive/Work/B.md", "b"),
            want("Archive/Work/Deep/C.md", "c"),
        ];
        let plan = plan(&store, &wanted);
        assert_eq!(plan.moved.len(), 3);
        assert_eq!(plan.embeddings(), 0);
        assert!(plan.drop.is_empty());
    }

    #[test]
    fn the_plan_does_not_depend_on_the_order_the_vault_is_listed_in() {
        let store = store(&[("A.md", "a"), ("B.md", "b")]);
        let forwards = plan(&store, &[want("A2.md", "a"), want("B2.md", "b")]);
        let backwards = plan(&store, &[want("B2.md", "b"), want("A2.md", "a")]);
        let mut a = forwards.moved.clone();
        let mut b = backwards.moved.clone();
        a.sort_by(|x, y| x.to.cmp(&y.to));
        b.sort_by(|x, y| x.to.cmp(&y.to));
        assert_eq!(a, b);
    }

    #[test]
    fn applying_a_plan_leaves_the_store_holding_exactly_the_live_notes() {
        // The catch-up loop in one step: plan, apply the bookkeeping, and what
        // is left needing a model is only the genuinely new work.
        let mut store = store(&[("A.md", "a"), ("Gone.md", "g"), ("Old/C.md", "c")]);
        let wanted = [
            want("A.md", "a"),
            want("New/C.md", "c"),
            want("Fresh.md", "f"),
        ];
        let work = plan(&store, &wanted);
        store.apply(&work);

        assert!(store.contains(&id("A.md")), "untouched note kept");
        assert!(store.contains(&id("New/C.md")), "moved note carried across");
        assert!(!store.contains(&id("Old/C.md")), "old id gone");
        assert!(!store.contains(&id("Gone.md")), "deleted note pruned");
        assert!(
            !store.contains(&id("Fresh.md")),
            "new note not embedded yet"
        );
        assert_eq!(paths(&work.embed), ["Fresh.md"]);

        // And once the embedding is done, the vault is level: a second plan is
        // empty, so a rescan on a quiet vault is free.
        store.insert(&id("Fresh.md"), Digest::of("f"), vec![vec![1.0, 0.0]]);
        assert!(plan(&store, &wanted).is_empty());
    }

    #[test]
    fn changing_the_model_empties_the_store() {
        // Vectors from two models are not comparable, and mixing them ranks
        // plausibly and wrongly, which is the worst way to be broken.
        let mut store = store(&[("A.md", "a")]);
        assert!(store.set_model("other-model"));
        assert!(store.is_empty());
        assert!(
            !store.set_model("other-model"),
            "no churn on the same model"
        );

        let plan = plan(&store, &[want("A.md", "a")]);
        assert_eq!(paths(&plan.embed), ["A.md"]);
    }

    #[test]
    fn nearest_ranks_by_similarity_and_honours_the_floor() {
        let mut store = Store::new("m");
        store.insert(&id("Close.md"), Digest::of("c"), vec![vec![1.0, 0.0]]);
        store.insert(&id("Middle.md"), Digest::of("m"), vec![vec![1.0, 1.0]]);
        store.insert(&id("Far.md"), Digest::of("f"), vec![vec![0.0, 1.0]]);

        let hits = store.nearest(&[1.0, 0.0], 0.0, 10);
        assert_eq!(
            paths(&hits.iter().map(|h| h.0.clone()).collect::<Vec<_>>()),
            ["Close.md", "Middle.md", "Far.md"]
        );

        // A query about nothing in the vault returns nothing, rather than the
        // least unrelated note.
        let filtered = store.nearest(&[1.0, 0.0], 0.9, 10);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, id("Close.md"));
    }

    #[test]
    fn a_note_scores_as_its_best_chunk_not_its_average() {
        // A long note with one paragraph about the query must beat a short one
        // that is vaguely on topic throughout.
        let mut store = Store::new("m");
        store.insert(
            &id("Long.md"),
            Digest::of("l"),
            vec![vec![0.0, 1.0], vec![1.0, 0.0], vec![0.0, 1.0]],
        );
        store.insert(&id("Vague.md"), Digest::of("v"), vec![vec![0.7, 0.7]]);

        let hits = store.nearest(&[1.0, 0.0], 0.0, 10);
        assert_eq!(hits[0].0, id("Long.md"));
        assert!((hits[0].1 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn vectors_of_the_wrong_width_do_not_match_and_do_not_panic() {
        // A store written by an older build, or half-migrated between models.
        let mut store = Store::new("m");
        store.insert(&id("A.md"), Digest::of("a"), vec![vec![1.0, 0.0, 0.0]]);
        assert!(store.nearest(&[1.0, 0.0], -1.0, 10).is_empty());
    }

    #[test]
    fn a_short_note_is_one_chunk_and_carries_its_title() {
        let chunks = chunks("Local inference", "The model runs on the 5090.");
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].starts_with("Local inference"));
        assert!(chunks[0].contains("runs on the 5090"));
    }

    #[test]
    fn headings_start_new_chunks_so_a_section_is_searchable_on_its_own() {
        let text = "Intro paragraph.\n\n# Servers\n\nThe 5090 holds one model.\n\n# Costs\n\nTokens are cheap.";
        let chunks = chunks("Notes", text);
        assert_eq!(chunks.len(), 3);
        assert!(chunks[1].contains("Servers"));
        assert!(chunks[2].contains("Costs"));
        // Every chunk names the note, since a chunk is read without it.
        assert!(chunks.iter().all(|chunk| chunk.starts_with("Notes")));
    }

    #[test]
    fn a_long_note_is_split_rather_than_truncated() {
        // The failure this prevents: the second half of a long note silently
        // stops being findable because the server truncated the input.
        let paragraph = "word ".repeat(300); // ~1500 chars
        let text = format!("{paragraph}\n\n{paragraph}\n\n{paragraph}");
        let chunks = chunks("Long", &text);
        assert!(chunks.len() >= 3, "got {} chunks", chunks.len());
        assert!(chunks
            .iter()
            .all(|chunk| chunk.chars().count() <= CHUNK_CHARS + "Long\n\n".len() + 1));
    }

    #[test]
    fn one_enormous_paragraph_is_cut_on_a_character_boundary() {
        let text = "é".repeat(CHUNK_CHARS * 2 + 10);
        let chunks = chunks("Wide", &text);
        assert!(chunks.len() >= 2);
        // Reassembling drops the titles, and what is left is the original.
        let joined: String = chunks
            .iter()
            .map(|chunk| chunk.trim_start_matches("Wide\n\n"))
            .collect();
        assert_eq!(joined, text);
    }

    #[test]
    fn a_note_is_never_split_into_more_chunks_than_the_cap() {
        let text = (0..100)
            .map(|n| format!("# Heading {n}\n\nBody {n}."))
            .collect::<Vec<_>>()
            .join("\n\n");
        assert_eq!(chunks("Huge", &text).len(), MAX_CHUNKS);
    }

    #[test]
    fn an_empty_note_still_has_its_title_to_match_on() {
        assert_eq!(chunks("Untitled thought", ""), ["Untitled thought"]);
        assert!(chunks("", "").is_empty());
    }

    // ---- catching up, with a model that is not a model ----

    use crate::note::Note;
    use std::cell::RefCell;

    /// An embedder that needs no server: each text becomes a vector of its own
    /// word counts over a fixed vocabulary, which is enough for "the same note
    /// embeds the same way" and "a different note does not".
    struct Fake {
        model: String,
        /// How many texts it has been asked to embed, so a test can assert that
        /// a move cost nothing.
        calls: RefCell<usize>,
        /// Refuse everything after this many texts, standing in for a server
        /// that goes away part way through.
        fails_after: Option<usize>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                model: "fake-embed".to_string(),
                calls: RefCell::new(0),
                fails_after: None,
            }
        }

        fn failing_after(texts: usize) -> Self {
            Self {
                fails_after: Some(texts),
                ..Self::new()
            }
        }

        fn calls(&self) -> usize {
            *self.calls.borrow()
        }
    }

    impl Embedder for Fake {
        fn model(&self) -> String {
            self.model.clone()
        }

        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
            let mut calls = self.calls.borrow_mut();
            if self.fails_after.is_some_and(|limit| *calls >= limit) {
                return Err(EmbedError("the server went away".into()));
            }
            *calls += texts.len();
            Ok(texts
                .iter()
                .map(|text| {
                    ["rust", "milk", "server"]
                        .iter()
                        .map(|word| text.matches(word).count() as f32 + 0.1)
                        .collect()
                })
                .collect())
        }
    }

    fn index_of(notes: &[(&str, &str)]) -> Index {
        let notes: Vec<Note> = notes
            .iter()
            .map(|(path, body)| Note::from_text(id(path), body))
            .collect();
        Index::build(&notes)
    }

    #[test]
    fn a_first_pass_embeds_the_vault_and_a_second_does_nothing() {
        let index = index_of(&[("A.md", "rust rust"), ("B.md", "milk")]);
        let embedder = Fake::new();
        let mut store = Store::default();

        let first = catch_up(&mut store, &index, &embedder);
        assert_eq!(first.embedded, 2);
        assert_eq!(store.len(), 2);
        assert_eq!(store.model, "fake-embed");

        // The property that makes it safe to run after every rescan: a quiet
        // vault is free, so nothing re-embeds on a timer.
        let calls = embedder.calls();
        let second = catch_up(&mut store, &index, &embedder);
        assert!(second.is_quiet(), "{second:?}");
        assert_eq!(embedder.calls(), calls, "a quiet vault called the model");
    }

    #[test]
    fn an_edit_re_embeds_only_the_note_that_changed() {
        let embedder = Fake::new();
        let mut store = Store::default();
        catch_up(
            &mut store,
            &index_of(&[("A.md", "rust"), ("B.md", "milk")]),
            &embedder,
        );
        let calls = embedder.calls();

        let edited = index_of(&[("A.md", "rust rust rust"), ("B.md", "milk")]);
        let report = catch_up(&mut store, &edited, &embedder);
        assert_eq!(report.embedded, 1);
        assert_eq!(embedder.calls(), calls + 1, "B was embedded again");
    }

    #[test]
    fn a_move_costs_no_model_time_at_all() {
        let embedder = Fake::new();
        let mut store = Store::default();
        catch_up(&mut store, &index_of(&[("Inbox/A.md", "rust")]), &embedder);
        let calls = embedder.calls();

        let moved = index_of(&[("Archive/Inbox/A.md", "rust")]);
        let report = catch_up(&mut store, &moved, &embedder);
        assert_eq!(report.moved, 1);
        assert_eq!(report.embedded, 0);
        assert_eq!(embedder.calls(), calls);
        assert!(store.contains(&id("Archive/Inbox/A.md")));
        assert!(!store.contains(&id("Inbox/A.md")));
    }

    #[test]
    fn a_deleted_note_stops_being_searchable() {
        // The pruning half. A vector left behind for a note that no longer
        // exists is worse than a missing one: the search returns a hit that
        // cannot be opened.
        let embedder = Fake::new();
        let mut store = Store::default();
        catch_up(
            &mut store,
            &index_of(&[("A.md", "rust"), ("B.md", "milk")]),
            &embedder,
        );

        let report = catch_up(&mut store, &index_of(&[("A.md", "rust")]), &embedder);
        assert_eq!(report.dropped, 1);
        assert!(!store.contains(&id("B.md")));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn the_whole_vault_disappearing_empties_the_store_rather_than_stranding_it() {
        let embedder = Fake::new();
        let mut store = Store::default();
        catch_up(
            &mut store,
            &index_of(&[("A.md", "rust"), ("B.md", "milk")]),
            &embedder,
        );
        let report = catch_up(&mut store, &index_of(&[]), &embedder);
        assert_eq!(report.dropped, 2);
        assert!(store.is_empty());
    }

    #[test]
    fn a_server_that_goes_away_leaves_a_usable_store_and_work_to_resume() {
        // Half the vault embedded, the other half pending. What is in the store
        // is still correct and still searchable; nothing is corrupt.
        let index = index_of(&[
            ("A.md", "rust"),
            ("B.md", "milk"),
            ("C.md", "server"),
            ("D.md", "rust milk"),
        ]);
        let mut store = Store::default();
        let dying = Fake::failing_after(2);

        let report = catch_up(&mut store, &index, &dying);
        assert_eq!(report.embedded, 2);
        assert_eq!(report.pending, 2);
        assert_eq!(store.len(), 2);

        // And the next pass, against a server that works, finishes the job
        // without redoing what was already done.
        let working = Fake::new();
        let resumed = catch_up(&mut store, &index, &working);
        assert_eq!(resumed.embedded, 2);
        assert_eq!(working.calls(), 2, "the first half was embedded again");
        assert_eq!(store.len(), 4);
        assert!(catch_up(&mut store, &index, &working).is_quiet());
    }

    #[test]
    fn swapping_the_model_re_embeds_the_vault_and_says_so() {
        let index = index_of(&[("A.md", "rust")]);
        let mut store = Store::default();
        catch_up(&mut store, &index, &Fake::new());

        let other = Fake {
            model: "a-different-model".into(),
            ..Fake::new()
        };
        let report = catch_up(&mut store, &index, &other);
        assert!(report.reset);
        assert_eq!(report.embedded, 1);
        assert_eq!(store.model, "a-different-model");
    }

    #[test]
    fn what_was_embedded_is_what_gets_found() {
        // End to end through the fake: catch up, then search, and the note
        // about the right subject comes back.
        let index = index_of(&[
            ("Rust.md", "rust rust rust"),
            ("Shopping.md", "milk milk milk"),
        ]);
        let embedder = Fake::new();
        let mut store = Store::default();
        catch_up(&mut store, &index, &embedder);

        let query = embedder.embed(&["rust".to_string()]).expect("embed")[0].clone();
        let hits = store.nearest(&query, 0.5, 5);
        assert_eq!(hits[0].0, id("Rust.md"));
    }

    #[test]
    fn a_store_round_trips_through_a_file() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("brain/vectors.json");

        let mut written = Store::new("bge-small");
        written.insert(&id("A.md"), Digest::of("a"), vec![vec![3.0, 4.0]]);
        written.save(&path).expect("save");

        let read = Store::load(&path);
        assert_eq!(read.model, "bge-small");
        // Vectors come back normalised, as they went in.
        assert_eq!(read.get(&id("A.md")).expect("A").chunks, [[0.6, 0.8]]);
        assert_eq!(read, written);
    }

    #[test]
    fn a_missing_or_corrupt_cache_is_an_empty_store_not_an_error() {
        // It is derived data. The recovery is to embed the vault again, which
        // is what the first run does anyway — an error dialog about a file in
        // ~/.cache would be noise about nothing lost.
        let directory = tempfile::tempdir().expect("temp dir");
        let missing = directory.path().join("nothing.json");
        assert!(Store::load(&missing).is_empty());

        let corrupt = directory.path().join("half.json");
        std::fs::write(&corrupt, "{\"model\":\"bge\",\"notes\":{\"A.md\":{\"dig").expect("write");
        assert!(Store::load(&corrupt).is_empty());
    }

    #[test]
    fn two_vaults_do_not_share_one_cache() {
        let one = default_store_path(std::path::Path::new("/home/someone/Notes"));
        let two = default_store_path(std::path::Path::new("/home/someone/Work"));
        assert_ne!(one, two);
        assert_eq!(one.parent(), two.parent());
        // And nothing is written inside the vault itself.
        assert!(!one.starts_with("/home/someone/Notes"));
    }

    #[test]
    fn the_digest_follows_the_text_that_is_embedded_not_the_file() {
        let same = digest_of("A", "one\n\ntwo");
        assert_eq!(same, digest_of("A", "one\n\ntwo"));
        assert_ne!(same, digest_of("A", "one\n\nthree"));
        // The title is part of what is embedded, so renaming a note is a real
        // change to its vectors — which is why a rename re-embeds and a move
        // does not.
        assert_ne!(same, digest_of("B", "one\n\ntwo"));
        // Whitespace around a block is not.
        assert_eq!(same, digest_of("A", "  one  \n\n  two  "));
    }
}
