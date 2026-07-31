//! The tag list.
//!
//! Nested tags (`#project/brain`) indent under their parents, and a parent
//! counts every note beneath it — the index does that arithmetic, so a tag with
//! no notes of its own still appears when its children have some.
//!
//! A `ListBox` rather than a `ListView` with a `TreeListModel`: a personal
//! vault has tens of tags, not thousands, and recycling rows costs more code
//! than it saves here.

use std::cell::RefCell;
use std::sync::OnceLock;

use gtk::glib::subclass::Signal;
use gtk::glib::{self, clone};
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct TagTree {
        pub list: RefCell<Option<gtk::ListBox>>,
        pub stack: RefCell<Option<gtk::Stack>>,
        /// The full tag each row selects, by row index.
        pub tags: RefCell<Vec<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TagTree {
        const NAME: &'static str = "BrainTagTree";
        type Type = super::TagTree;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for TagTree {
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
                vec![Signal::builder("tag-chosen")
                    .param_types([String::static_type()])
                    .build()]
            })
        }
    }

    impl WidgetImpl for TagTree {}
}

glib::wrapper! {
    pub struct TagTree(ObjectSubclass<imp::TagTree>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for TagTree {
    fn default() -> Self {
        Self::new()
    }
}

impl TagTree {
    pub fn new() -> Self {
        glib::Object::new()
    }

    fn build(&self) {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .build();
        list.add_css_class("navigation-sidebar");

        list.connect_row_activated(clone!(
            #[weak(rename_to = tree)]
            self,
            move |_, row| {
                let index = row.index();
                if index < 0 {
                    return;
                }
                let tag = tree.imp().tags.borrow().get(index as usize).cloned();
                if let Some(tag) = tag {
                    tree.emit_by_name::<()>("tag-chosen", &[&tag]);
                }
            }
        ));

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();

        let empty = adw::StatusPage::builder()
            .icon_name("tag-symbolic")
            .title("No Tags")
            .description("Write #tag in a note, or add tags to its frontmatter.")
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

    /// Show every tag, in the index's order, with its note count.
    pub fn set_tags(&self, tags: &[(String, usize)]) {
        let imp = self.imp();
        let (Some(list), Some(stack)) = (imp.list.borrow().clone(), imp.stack.borrow().clone())
        else {
            return;
        };

        while let Some(child) = list.first_child() {
            list.remove(&child);
        }

        for (tag, count) in tags {
            let depth = tag.matches('/').count() as i32;
            // Only the last segment is shown: "project/brain" under "project"
            // reads as "brain", the way a folder tree does.
            let leaf = tag.rsplit('/').next().unwrap_or(tag);

            let label = gtk::Label::builder()
                .label(format!("#{leaf}"))
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                .build();

            let counter = gtk::Label::builder().label(count.to_string()).build();
            counter.add_css_class("dimmed");
            counter.add_css_class("numeric");

            let row_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(6)
                .margin_top(8)
                .margin_bottom(8)
                .margin_start(12 + depth * 16)
                .margin_end(12)
                .build();
            row_box.append(&label);
            row_box.append(&counter);

            let row = gtk::ListBoxRow::builder().child(&row_box).build();
            list.append(&row);
        }

        imp.tags
            .replace(tags.iter().map(|(tag, _)| tag.clone()).collect());
        stack.set_visible_child_name(if tags.is_empty() { "empty" } else { "list" });
    }

    /// Highlight a tag without emitting `tag-chosen`.
    pub fn select(&self, tag: Option<&str>) {
        let imp = self.imp();
        let Some(list) = imp.list.borrow().clone() else {
            return;
        };
        match tag.and_then(|tag| imp.tags.borrow().iter().position(|it| it == tag)) {
            Some(index) => {
                if let Some(row) = list.row_at_index(index as i32) {
                    list.select_row(Some(&row));
                }
            }
            None => list.select_row(gtk::ListBoxRow::NONE),
        }
    }
}
