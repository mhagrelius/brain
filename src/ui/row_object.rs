//! A note projected into a `GObject`, for the sidebar's `ListView`.
//!
//! Deliberately flat, pre-formatted strings. This is a *projection*, not a
//! second source of truth: the vault holds the notes, and one of these is made
//! from a note whenever the list is rebuilt. Nothing reads a note's content
//! back out of it.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::model::note::NoteId;

mod imp {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::NoteObject)]
    pub struct NoteObject {
        /// The vault-relative path, which is the note's identity.
        #[property(get, set)]
        pub id: RefCell<String>,
        #[property(get, set)]
        pub title: RefCell<String>,
        /// The containing folder, or empty at the vault root.
        #[property(get, set)]
        pub folder: RefCell<String>,
        #[property(get, set)]
        pub excerpt: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NoteObject {
        const NAME: &'static str = "BrainNoteObject";
        type Type = super::NoteObject;
    }

    #[glib::derived_properties]
    impl ObjectImpl for NoteObject {}
}

glib::wrapper! {
    pub struct NoteObject(ObjectSubclass<imp::NoteObject>);
}

impl NoteObject {
    pub fn new(id: &NoteId, excerpt: &str) -> Self {
        glib::Object::builder()
            .property("id", id.as_str())
            .property("title", id.title())
            .property("folder", id.folder().unwrap_or(""))
            .property("excerpt", excerpt)
            .build()
    }

    pub fn note_id(&self) -> NoteId {
        NoteId::from_relative(self.id())
    }
}
