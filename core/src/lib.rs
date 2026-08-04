//! Brain's core: the vault, the notes, the index and the search.
//!
//! Nothing here links a UI toolkit, and nothing here draws. `cargo test` runs
//! the whole of it with no display, which is why as much of Brain's behaviour
//! as possible is pushed down into it.
//!
//! The shell — `brain` itself — re-exports this crate as `brain::model`, so
//! every `crate::model::…` path in the application still means what it always
//! did.

pub mod bm25;
pub mod config;
pub mod frontmatter;
pub mod index;
// The Markdown scanner lives in its own crate now — Stickies and Familiar read
// the same one. Re-exported under the path it has always had here, so every
// `model::markdown::…` call site still means what it did.
pub use quill as markdown;
pub mod note;
pub mod search;
pub mod semantic;
pub mod tree;
pub mod vault;
