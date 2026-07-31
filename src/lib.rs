//! Brain: a Markdown notebook for GNOME.
//!
//! Two halves. `model/` links no GTK and is exercised by `cargo test` with no
//! display: it is the vault, the notes, the scanner and the index. `ui/` is the
//! only half that knows a window exists, and `ui::BrainApplication` is the only
//! thing that writes a file.

pub mod model;
pub mod ui;

pub const APP_ID: &str = "us.hagreli.Brain";
