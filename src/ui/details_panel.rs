//! The right-hand pane's "Details" view: what a note is, and what can be done
//! to it.
//!
//! Two jobs, and the second is the reason it exists. The properties half says
//! what the open note is — where it lives, when it was written, how long it is,
//! what it is tagged. The formatting half is a menu of the Markdown the editor
//! understands: each button writes its syntax *and shows it*, so the pane
//! teaches the format rather than hiding it behind a toolbar.
//!
//! It applies nothing itself. It reports which formatting was asked for, and
//! the window passes that to the editor.

use std::cell::RefCell;
use std::sync::OnceLock;

use adw::prelude::*;
use gtk::glib::subclass::Signal;
use gtk::glib::{self, clone};
use gtk::subclass::prelude::*;

use crate::model::markdown::Format;
use crate::model::note::NoteId;

/// The formatting on offer, in the order it is shown. Grouped the way someone
/// writing would reach for it: emphasis, then structure, then blocks.
/// Three to a row: at the width of this pane a fourth column squeezes the
/// two-line labels until they ellipsize into uselessness.
const FORMATS: &[&[Format]] = &[
    &[Format::Bold, Format::Italic, Format::Strikethrough],
    &[Format::Heading(1), Format::Heading(2), Format::Heading(3)],
    &[Format::Bullet, Format::Task, Format::Quote],
    &[Format::WikiLink, Format::Link, Format::Code],
    &[Format::CodeBlock, Format::Table, Format::Rule],
];

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct DetailsPanel {
        pub stack: RefCell<Option<gtk::Stack>>,
        pub title: RefCell<Option<gtk::Label>>,
        pub properties: RefCell<Option<adw::PreferencesGroup>>,
        pub formatting: RefCell<Option<adw::PreferencesGroup>>,
        pub rows: RefCell<Vec<adw::ActionRow>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DetailsPanel {
        const NAME: &'static str = "BrainDetailsPanel";
        type Type = super::DetailsPanel;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for DetailsPanel {
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
                vec![Signal::builder("format-requested")
                    .param_types([String::static_type()])
                    .build()]
            })
        }
    }

    impl WidgetImpl for DetailsPanel {}
}

glib::wrapper! {
    pub struct DetailsPanel(ObjectSubclass<imp::DetailsPanel>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for DetailsPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl DetailsPanel {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// The name a `Format` answers to over the signal.
    ///
    /// A string rather than a boxed type, because a GObject signal carries
    /// plain values and this is the whole of what has to cross.
    pub fn format_name(format: Format) -> String {
        match format {
            Format::Heading(level) => format!("heading{level}"),
            other => other.label().to_lowercase().replace(' ', "-"),
        }
    }

    pub fn format_from_name(name: &str) -> Option<Format> {
        FORMATS
            .iter()
            .flat_map(|group| group.iter())
            .copied()
            .find(|format| Self::format_name(*format) == name)
    }

    fn build(&self) {
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .margin_top(12)
            .margin_bottom(18)
            .margin_start(12)
            .margin_end(12)
            .build();

        // ---- properties ----
        let properties = adw::PreferencesGroup::builder().title("Note").build();
        let mut rows = Vec::new();
        for title in ["Folder", "Tags", "Words", "Created", "Updated"] {
            let row = adw::ActionRow::builder().title(title).build();
            row.add_css_class("property");
            properties.add(&row);
            rows.push(row);
        }
        content.append(&properties);

        // ---- formatting ----
        let formatting = adw::PreferencesGroup::builder()
            .title("Formatting")
            .description("Applies to the selection, or inserts at the cursor.")
            .build();

        let grid = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .build();
        for group in FORMATS {
            let row = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(6)
                .homogeneous(true)
                .build();
            for format in *group {
                row.append(&self.format_button(*format));
            }
            grid.append(&row);
        }
        formatting.add(&grid);
        content.append(&formatting);

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&content)
            .build();

        // Nothing open is a real state: the properties would be blank and the
        // formatting would have nowhere to go.
        let empty = adw::StatusPage::builder()
            .title("No Note Open")
            .description("Open a note to see its details.")
            .build();
        empty.add_css_class("compact");

        let stack = gtk::Stack::new();
        stack.add_named(&scroller, Some("details"));
        stack.add_named(&empty, Some("empty"));
        stack.set_visible_child_name("empty");
        stack.set_parent(self);
        self.set_vexpand(true);

        self.imp().stack.replace(Some(stack));
        self.imp().properties.replace(Some(properties));
        self.imp().formatting.replace(Some(formatting));
        self.imp().rows.replace(rows);
    }

    /// Grey the formatting out, for reading mode: buttons that write Markdown
    /// into a note that will not accept it would be lying.
    pub fn set_formatting_enabled(&self, enabled: bool) {
        if let Some(group) = self.imp().formatting.borrow().as_ref() {
            group.set_sensitive(enabled);
        }
    }

    /// One formatting button: what it does, and the syntax it writes.
    fn format_button(&self, format: Format) -> gtk::Button {
        let label = gtk::Label::builder()
            .label(format.label())
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        label.add_css_class("caption-heading");

        let syntax = gtk::Label::builder()
            .label(format.syntax())
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        syntax.add_css_class("caption");
        syntax.add_css_class("dimmed");
        syntax.add_css_class("monospace");

        let stack = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(1)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        stack.append(&label);
        stack.append(&syntax);

        let button = gtk::Button::builder().child(&stack).build();
        button.add_css_class("flat");
        button.add_css_class("card");
        button.set_tooltip_text(Some(&format!(
            "{} — writes {}",
            format.label(),
            format.syntax()
        )));

        let name = Self::format_name(format);
        button.connect_clicked(clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.emit_by_name::<()>("format-requested", &[&name])
        ));
        button
    }

    /// Show a note's properties, or none.
    pub fn set_note(
        &self,
        note: Option<&NoteId>,
        tags: &[String],
        words: usize,
        created: Option<String>,
        updated: Option<String>,
    ) {
        let imp = self.imp();
        let Some(stack) = imp.stack.borrow().clone() else {
            return;
        };

        let Some(id) = note else {
            stack.set_visible_child_name("empty");
            return;
        };
        stack.set_visible_child_name("details");

        if let Some(group) = imp.properties.borrow().as_ref() {
            group.set_title(id.title());
        }

        let values = [
            id.folder().unwrap_or("Vault root").to_string(),
            if tags.is_empty() {
                "None".to_string()
            } else {
                tags.iter()
                    .map(|tag| format!("#{tag}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            },
            match words {
                1 => "1 word".to_string(),
                other => format!("{other} words"),
            },
            created.unwrap_or_else(|| "Not recorded".to_string()),
            updated.unwrap_or_else(|| "Not recorded".to_string()),
        ];

        for (row, value) in imp.rows.borrow().iter().zip(values) {
            row.set_subtitle(&value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_format_survives_the_round_trip_through_its_name() {
        // The name is what crosses the signal. One that does not come back is
        // a button that silently does nothing.
        for format in FORMATS.iter().flat_map(|group| group.iter()).copied() {
            let name = DetailsPanel::format_name(format);
            assert_eq!(
                DetailsPanel::format_from_name(&name),
                Some(format),
                "{format:?} did not survive as {name:?}"
            );
        }
    }

    #[test]
    fn format_names_are_unique() {
        // Two formats sharing a name would make one of the buttons do the
        // other's job.
        let mut names: Vec<String> = FORMATS
            .iter()
            .flat_map(|group| group.iter())
            .map(|format| DetailsPanel::format_name(*format))
            .collect();
        let count = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), count, "duplicate format names: {names:?}");
    }
}
