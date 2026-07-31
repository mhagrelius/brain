//! The completion list that opens when you type `[[`.
//!
//! It knows nothing about the vault. The editor tells it what the user has
//! typed so far and hands it a list of candidates; it reports which one was
//! chosen. Deciding what a candidate *is* belongs to the index.

use std::cell::RefCell;
use std::sync::OnceLock;

use gtk::glib::subclass::Signal;
use gtk::glib::{self, clone};
use gtk::prelude::*;
use gtk::subclass::prelude::*;

/// Candidates shown at once. More than this and the popover becomes a window
/// with a scrollbar, which is not what a completion is for.
const VISIBLE: usize = 8;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct LinkPopover {
        pub popover: RefCell<Option<gtk::Popover>>,
        pub list: RefCell<Option<gtk::ListBox>>,
        /// The candidates as shown, so a keyboard selection can name one.
        pub candidates: RefCell<Vec<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LinkPopover {
        const NAME: &'static str = "BrainLinkPopover";
        type Type = super::LinkPopover;
        type ParentType = glib::Object;
    }

    impl ObjectImpl for LinkPopover {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![Signal::builder("chosen")
                    .param_types([String::static_type()])
                    .build()]
            })
        }
    }
}

glib::wrapper! {
    pub struct LinkPopover(ObjectSubclass<imp::LinkPopover>);
}

impl LinkPopover {
    /// Build a popover parented to `anchor`, which owns it.
    pub fn new(anchor: &impl IsA<gtk::Widget>) -> Self {
        let object: Self = glib::Object::new();

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Browse)
            .build();
        list.add_css_class("boxed-list");

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(280)
            .child(&list)
            .build();

        let popover = gtk::Popover::builder()
            .autohide(false) // the editor keeps the focus; typing must not close it
            .has_arrow(false)
            .position(gtk::PositionType::Bottom)
            .child(&scroller)
            .build();
        popover.set_parent(anchor);
        popover.add_css_class("menu");

        list.connect_row_activated(clone!(
            #[weak(rename_to = object)]
            object,
            move |_, row| {
                let index = row.index();
                if index < 0 {
                    return;
                }
                let chosen = object
                    .imp()
                    .candidates
                    .borrow()
                    .get(index as usize)
                    .cloned();
                if let Some(chosen) = chosen {
                    object.emit_by_name::<()>("chosen", &[&chosen]);
                }
            }
        ));

        object.imp().popover.replace(Some(popover));
        object.imp().list.replace(Some(list));
        object
    }

    pub fn is_open(&self) -> bool {
        self.imp()
            .popover
            .borrow()
            .as_ref()
            .is_some_and(|popover| popover.is_visible())
    }

    /// Show `candidates`, pointing at `at` in the anchor's coordinates.
    ///
    /// An empty list closes the popover: offering a completion with nothing in
    /// it is a box that swallows the next keystroke for no reason.
    pub fn show(&self, candidates: &[String], at: &gtk::gdk::Rectangle) {
        let imp = self.imp();
        let (Some(popover), Some(list)) = (imp.popover.borrow().clone(), imp.list.borrow().clone())
        else {
            return;
        };

        if candidates.is_empty() {
            popover.popdown();
            imp.candidates.replace(Vec::new());
            return;
        }

        while let Some(child) = list.first_child() {
            list.remove(&child);
        }

        let shown: Vec<String> = candidates.iter().take(VISIBLE).cloned().collect();
        for candidate in &shown {
            let label = gtk::Label::builder()
                .label(candidate)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                .margin_top(8)
                .margin_bottom(8)
                .margin_start(12)
                .margin_end(12)
                .build();
            list.append(&label);
        }
        imp.candidates.replace(shown);

        if let Some(first) = list.row_at_index(0) {
            list.select_row(Some(&first));
        }

        popover.set_pointing_to(Some(at));
        popover.popup();
    }

    pub fn hide(&self) {
        if let Some(popover) = self.imp().popover.borrow().as_ref() {
            popover.popdown();
        }
        self.imp().candidates.replace(Vec::new());
    }

    /// Move the selection. `delta` is rows, positive downwards.
    pub fn move_selection(&self, delta: i32) {
        let Some(list) = self.imp().list.borrow().clone() else {
            return;
        };
        let count = self.imp().candidates.borrow().len() as i32;
        if count == 0 {
            return;
        }
        let current = list.selected_row().map(|row| row.index()).unwrap_or(0);
        // Wrapping, because a completion list is a ring: pressing up on the
        // first item should reach the last, not do nothing.
        let next = (current + delta).rem_euclid(count);
        if let Some(row) = list.row_at_index(next) {
            list.select_row(Some(&row));
        }
    }

    /// Take the selected candidate, if the popover is open.
    pub fn selected(&self) -> Option<String> {
        if !self.is_open() {
            return None;
        }
        let list = self.imp().list.borrow().clone()?;
        let index = list.selected_row().map(|row| row.index())?;
        self.imp()
            .candidates
            .borrow()
            .get(index.max(0) as usize)
            .cloned()
    }

    /// Unparent the popover, for the anchor's `dispose`.
    pub fn destroy(&self) {
        if let Some(popover) = self.imp().popover.take() {
            popover.unparent();
        }
    }
}
