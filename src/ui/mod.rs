//! The half that knows a window exists.
//!
//! Widget trees are built in Rust — no `.ui` XML, no Blueprint, no GResource.
//! The structure of a pane is then readable in the same file as the behaviour
//! that drives it, which for an app this size is worth more than a designer
//! could give back.

mod application;
mod attachments;
mod backlinks_panel;
mod details_panel;
mod editor;
mod embedder;
mod highlight;
mod link_popover;
mod palette;
mod row_object;
mod sidebar;
mod tag_tree;
mod watcher;
mod window;

pub use application::BrainApplication;
pub use backlinks_panel::BacklinksPanel;
pub use details_panel::DetailsPanel;
pub use editor::Editor;
pub use embedder::{Llama, DEFAULT_EMBEDDING_URL};
pub use link_popover::LinkPopover;
pub use palette::{Hit, Mode, Palette};
pub use row_object::RowObject;
pub use sidebar::{Dragged, Sidebar};
pub use tag_tree::TagTree;
pub use watcher::Watcher;
pub use window::BrainWindow;

/// The application stylesheet, compiled in.
pub const STYLE: &str = include_str!("style.css");

/// Load the stylesheet at application priority, above the theme and below the
/// user's own overrides.
pub fn load_stylesheet(display: &gtk::gdk::Display) {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(STYLE);
    gtk::style_context_add_provider_for_display(
        display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}
