//! The pane listing every note that links to the open one.
//!
//! Each row is a note and the line the link was written on, stripped of its
//! markup — the pane exists to be read, so a backlink you have to open to
//! understand has failed at its job.

use std::cell::RefCell;
use std::sync::OnceLock;

use gtk::glib::subclass::Signal;
use gtk::glib::{self, clone};
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::model::note::NoteId;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct BacklinksPanel {
        pub list: RefCell<Option<gtk::ListBox>>,
        pub stack: RefCell<Option<gtk::Stack>>,
        /// The note each row points at, by row index.
        pub targets: RefCell<Vec<NoteId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BacklinksPanel {
        const NAME: &'static str = "BrainBacklinksPanel";
        type Type = super::BacklinksPanel;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for BacklinksPanel {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }

        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![Signal::builder("note-chosen")
                    .param_types([String::static_type()])
                    .build()]
            })
        }
    }

    impl WidgetImpl for BacklinksPanel {}
}

glib::wrapper! {
    pub struct BacklinksPanel(ObjectSubclass<imp::BacklinksPanel>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for BacklinksPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl BacklinksPanel {
    pub fn new() -> Self {
        glib::Object::new()
    }

    fn build(&self) {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .margin_top(12)
            .margin_start(12)
            .margin_end(12)
            .margin_bottom(12)
            .valign(gtk::Align::Start)
            .build();
        list.add_css_class("boxed-list");

        list.connect_row_activated(clone!(
            #[weak(rename_to = panel)]
            self,
            move |_, row| {
                let index = row.index();
                if index < 0 {
                    return;
                }
                let target = panel.imp().targets.borrow().get(index as usize).cloned();
                if let Some(target) = target {
                    panel.emit_by_name::<()>("note-chosen", &[&target.as_str().to_string()]);
                }
            }
        ));

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();

        // Nothing linking here is the common case for a new note, and is not a
        // failure — say so quietly rather than showing an empty box.
        let empty = adw::StatusPage::builder()
            .icon_name("insert-link-symbolic")
            .title("No Backlinks")
            .description("Notes that link here will appear in this pane.")
            .build();
        empty.add_css_class("compact");

        let stack = gtk::Stack::new();
        stack.add_named(&scroller, Some("list"));
        stack.add_named(&empty, Some("empty"));
        stack.set_visible_child_name("empty");
        stack.set_parent(self);
        self.set_vexpand(true);

        self.imp().list.replace(Some(list));
        self.imp().stack.replace(Some(stack));
    }

    /// Show the notes linking here, each with the line it was linked from.
    pub fn set_backlinks(&self, backlinks: &[(NoteId, String)]) {
        let imp = self.imp();
        let (Some(list), Some(stack)) = (imp.list.borrow().clone(), imp.stack.borrow().clone())
        else {
            return;
        };

        while let Some(child) = list.first_child() {
            list.remove(&child);
        }

        for (id, context) in backlinks {
            let title = gtk::Label::builder()
                .label(id.title())
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            title.add_css_class("note-row-title");

            let row_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(2)
                .margin_top(8)
                .margin_bottom(8)
                .margin_start(12)
                .margin_end(12)
                .build();
            row_box.append(&title);

            if !context.is_empty() {
                let line = gtk::Label::builder()
                    .label(context)
                    .xalign(0.0)
                    .wrap(true)
                    .lines(2)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .build();
                line.add_css_class("note-row-excerpt");
                line.add_css_class("dimmed");
                row_box.append(&line);
            }

            let row = gtk::ListBoxRow::builder().child(&row_box).build();
            list.append(&row);
        }

        imp.targets
            .replace(backlinks.iter().map(|(id, _)| id.clone()).collect());

        stack.set_visible_child_name(if backlinks.is_empty() {
            "empty"
        } else {
            "list"
        });
    }
}
