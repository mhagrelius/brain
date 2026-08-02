pub mod bm25;
pub mod config;
pub mod frontmatter;
pub mod index;
// The Markdown scanner lives in its own crate now — Stickies and Familiar read
// the same one. Re-exported under the path it has always had here, so every
// `crate::model::markdown::…` in the app still means what it did.
pub use quill as markdown;
pub mod note;
pub mod search;
pub mod semantic;
pub mod tree;
pub mod vault;
