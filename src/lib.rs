//! Brain: a Markdown notebook for GNOME.
//!
//! Two halves, and they are now two crates. `brain-core` links no GTK and is
//! exercised by `cargo test` with no display: it is the vault, the notes, the
//! scanner and the index. `ui/` is the only half that knows a window exists,
//! and `ui::BrainApplication` is the only thing that writes a file.
//!
//! The core is re-exported here under the name it has always had, so every
//! `model::…` path in the application and its tests still means what it did —
//! the same trick `brain_core` itself plays with `quill`.

pub use brain_core as model;
pub mod ui;

pub const APP_ID: &str = "us.hagreli.Brain";
