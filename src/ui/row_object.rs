//! A sidebar row projected into a `GObject`, for the `ListView`.
//!
//! Deliberately flat, pre-formatted strings. This is a *projection*, not a
//! second source of truth: the vault holds the notes, [`Row`] says what the
//! sidebar looks like, and one of these is made from each row whenever the list
//! is rebuilt. Nothing reads a note's content back out of it.
//!
//! Folders and notes are one type rather than two, because a `ListView` takes
//! one model and the alternative — a store of `glib::Object` downcast in
//! `bind` — moves the same branch somewhere it cannot be read.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::model::note::NoteId;
use crate::model::tree::Row;

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::RowObject)]
    pub struct RowObject {
        /// `note` or `folder`, which is what the factory branches on.
        #[property(get, set)]
        pub kind: RefCell<String>,
        /// The vault-relative path: a note's identity, or a folder's.
        #[property(get, set)]
        pub id: RefCell<String>,
        /// The note's title, or the folder's last segment.
        #[property(get, set)]
        pub title: RefCell<String>,
        /// The containing folder, shown only where the indent cannot say it —
        /// which is search results, where the tree is flattened.
        #[property(get, set)]
        pub folder: RefCell<String>,
        #[property(get, set)]
        pub excerpt: RefCell<String>,
        /// How deep in the tree, in levels. Drawn as a left margin.
        #[property(get, set)]
        pub depth: Cell<u32>,
        /// Notes anywhere beneath a folder. Zero on a note.
        #[property(get, set)]
        pub count: Cell<u32>,
        #[property(get, set)]
        pub expanded: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RowObject {
        const NAME: &'static str = "BrainRowObject";
        type Type = super::RowObject;
    }

    #[glib::derived_properties]
    impl ObjectImpl for RowObject {}
}

glib::wrapper! {
    pub struct RowObject(ObjectSubclass<imp::RowObject>);
}

impl RowObject {
    /// A note as it appears in the tree, where the indent says which folder it
    /// is in.
    pub fn note(id: &NoteId, excerpt: &str, depth: u32) -> Self {
        glib::Object::builder()
            .property("kind", "note")
            .property("id", id.as_str())
            .property("title", id.title())
            .property("folder", "")
            .property("excerpt", excerpt)
            .property("depth", depth)
            .build()
    }

    /// A note as a search result: no indent, and the folder spelled out, since
    /// two notes answering to one title are told apart by nothing else.
    pub fn result(id: &NoteId, excerpt: &str) -> Self {
        glib::Object::builder()
            .property("kind", "note")
            .property("id", id.as_str())
            .property("title", id.title())
            .property("folder", id.folder().unwrap_or(""))
            .property("excerpt", excerpt)
            .build()
    }

    pub fn for_folder(path: &str, name: &str, depth: u32, count: u32, expanded: bool) -> Self {
        glib::Object::builder()
            .property("kind", "folder")
            .property("id", path)
            .property("title", name)
            .property("folder", "")
            .property("excerpt", "")
            .property("depth", depth)
            .property("count", count)
            .property("expanded", expanded)
            .build()
    }

    pub fn from_row(row: &Row) -> Self {
        match row {
            Row::Note { id, excerpt, depth } => Self::note(id, excerpt, *depth as u32),
            Row::Folder {
                path,
                name,
                depth,
                notes,
                expanded,
            } => Self::for_folder(path, name, *depth as u32, *notes as u32, *expanded),
        }
    }

    pub fn is_folder(&self) -> bool {
        self.kind() == "folder"
    }

    pub fn note_id(&self) -> NoteId {
        NoteId::from_relative(self.id())
    }
}
