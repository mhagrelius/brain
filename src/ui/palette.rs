//! The search palette: `Ctrl+K` by title, `Ctrl+Shift+F` by text.
//!
//! One dialog with two modes rather than two dialogs, because they differ in
//! what they search and in nothing else — the typing, the ranking, the arrow
//! keys and the Return are the same either way, and you often want the other
//! one having already typed the query.
//!
//! The palette holds no vault. It reports what was typed and is handed results
//! back, the same arrangement as the link completion.

use std::cell::{Cell, RefCell};
use std::sync::OnceLock;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib::subclass::Signal;
use gtk::glib::{self, clone};

// `Mode` and `Hit` are the notebook's — what a query is matched against and
// what came back are the same questions on any platform, so they live beside
// the search that answers them rather than beside the dialog that draws it.
use crate::model::notebook::{Hit, Mode};

/// What the entry says before anything is typed. A free function rather than a
/// method, because the wording is this shell's and the type is not.
fn placeholder(mode: Mode) -> &'static str {
    match mode {
        Mode::Title => "Go to note…",
        Mode::Text => "Search all notes…",
    }
}

/// Rows shown at once. Past this the list stops being scannable and the ranking
/// is doing the work anyway.
const LIMIT: usize = 30;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Palette {
        pub entry: RefCell<Option<gtk::SearchEntry>>,
        pub list: RefCell<Option<gtk::ListBox>>,
        pub stack: RefCell<Option<gtk::Stack>>,
        pub scroller: RefCell<Option<gtk::ScrolledWindow>>,
        pub empty: RefCell<Option<adw::StatusPage>>,
        pub toggles: RefCell<Option<adw::ToggleGroup>>,
        pub mode: Cell<Mode>,
        /// The note each row points at, by index.
        pub ids: RefCell<Vec<String>>,
        /// Set while the mode is being changed in code, so the toggle handler
        /// does not re-run the search it was told about.
        pub switching: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Palette {
        const NAME: &'static str = "BrainPalette";
        type Type = super::Palette;
        type ParentType = adw::Dialog;
    }

    impl ObjectImpl for Palette {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    // The query or the mode changed. The handler is expected to
                    // call `set_hits` before returning.
                    Signal::builder("query-changed")
                        .param_types([String::static_type()])
                        .build(),
                    // A note was picked. Carries its id.
                    Signal::builder("chosen")
                        .param_types([String::static_type()])
                        .build(),
                ]
            })
        }
    }

    impl WidgetImpl for Palette {}
    impl AdwDialogImpl for Palette {}
}

glib::wrapper! {
    pub struct Palette(ObjectSubclass<imp::Palette>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Palette {
    fn default() -> Self {
        Self::new()
    }
}

impl Palette {
    pub fn new() -> Self {
        glib::Object::new()
    }

    fn build(&self) {
        self.set_content_width(640);
        self.set_content_height(520);
        self.set_title("Search");

        let entry = gtk::SearchEntry::builder()
            .placeholder_text(placeholder(Mode::Title))
            .hexpand(true)
            .build();

        let toggles = adw::ToggleGroup::new();
        toggles.add(adw::Toggle::builder().name("title").label("Titles").build());
        toggles.add(adw::Toggle::builder().name("text").label("Text").build());
        toggles.set_active_name(Some("title"));

        let header = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        header.append(&entry);
        header.append(&toggles);

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Browse)
            .build();
        list.add_css_class("navigation-sidebar");

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();
        let scroller_for_stack = scroller.clone();

        let empty = adw::StatusPage::builder()
            .icon_name("system-search-symbolic")
            .title("No Matches")
            .build();
        empty.add_css_class("compact");

        let stack = gtk::Stack::new();
        stack.add_named(&scroller_for_stack, Some("list"));
        stack.add_named(&empty, Some("empty"));

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&header);
        content.append(&stack);
        self.set_child(Some(&content));

        entry.connect_search_changed(clone!(
            #[weak(rename_to = palette)]
            self,
            move |_| palette.query_changed()
        ));

        // Return from the entry takes the highlighted row, so a search is one
        // uninterrupted piece of typing.
        entry.connect_activate(clone!(
            #[weak(rename_to = palette)]
            self,
            move |_| palette.choose_selected()
        ));

        list.connect_row_activated(clone!(
            #[weak(rename_to = palette)]
            self,
            move |_, row| {
                let index = row.index();
                if index < 0 {
                    return;
                }
                let id = palette.imp().ids.borrow().get(index as usize).cloned();
                if let Some(id) = id {
                    palette.emit_by_name::<()>("chosen", &[&id]);
                    palette.close();
                }
            }
        ));

        toggles.connect_active_name_notify(clone!(
            #[weak(rename_to = palette)]
            self,
            move |toggles| {
                if palette.imp().switching.get() {
                    return;
                }
                let mode = match toggles.active_name().as_deref() {
                    Some("text") => Mode::Text,
                    _ => Mode::Title,
                };
                palette.imp().mode.set(mode);
                if let Some(entry) = palette.imp().entry.borrow().as_ref() {
                    entry.set_placeholder_text(Some(placeholder(mode)));
                }
                palette.query_changed();
            }
        ));

        // The arrow keys drive the list while the focus stays in the entry,
        // which is the whole point of a palette.
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(clone!(
            #[weak(rename_to = palette)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, _| {
                use gtk::gdk::Key;
                match key {
                    Key::Down => {
                        palette.move_selection(1);
                        glib::Propagation::Stop
                    }
                    Key::Up => {
                        palette.move_selection(-1);
                        glib::Propagation::Stop
                    }
                    _ => glib::Propagation::Proceed,
                }
            }
        ));
        entry.add_controller(keys);

        self.imp().entry.replace(Some(entry));
        self.imp().list.replace(Some(list));
        self.imp().stack.replace(Some(stack));
        self.imp().scroller.replace(Some(scroller));
        self.imp().empty.replace(Some(empty));
        self.imp().toggles.replace(Some(toggles));
    }

    pub fn mode(&self) -> Mode {
        self.imp().mode.get()
    }

    pub fn query(&self) -> String {
        self.imp()
            .entry
            .borrow()
            .as_ref()
            .map(|entry| entry.text().to_string())
            .unwrap_or_default()
    }

    /// Open in a mode, keeping whatever was already typed.
    ///
    /// Keeping it means `Ctrl+Shift+F` after a fruitless `Ctrl+K` searches the
    /// text for what you already typed, rather than making you type it again.
    pub fn open(&self, parent: &impl IsA<gtk::Widget>, mode: Mode) {
        let imp = self.imp();
        imp.mode.set(mode);
        imp.switching.set(true);
        if let Some(toggles) = imp.toggles.borrow().as_ref() {
            toggles.set_active_name(Some(match mode {
                Mode::Title => "title",
                Mode::Text => "text",
            }));
        }
        imp.switching.set(false);

        if let Some(entry) = imp.entry.borrow().as_ref() {
            entry.set_placeholder_text(Some(placeholder(mode)));
            entry.select_region(0, -1);
        }

        self.present(Some(parent));
        if let Some(entry) = imp.entry.borrow().as_ref() {
            entry.grab_focus();
        }
        self.query_changed();
    }

    fn query_changed(&self) {
        let query = self.query();
        // Synchronous: the handler calls `set_hits` before this returns.
        self.emit_by_name::<()>("query-changed", &[&query]);
    }

    /// Ask for the results again, because something behind them changed.
    ///
    /// The semantic half of a text search arrives after the lexical half — the
    /// query has to reach a model and come back — so the palette answers
    /// instantly with what it has and is told to ask again when the rest lands.
    /// A no-op when nothing has been typed, so an embedding that arrives after
    /// the palette was cleared does not repopulate it.
    pub fn refresh(&self) {
        if !self.query().is_empty() {
            self.query_changed();
        }
    }

    /// Show these results.
    pub fn set_hits(&self, hits: &[Hit]) {
        let imp = self.imp();
        let (Some(list), Some(stack), Some(empty)) = (
            imp.list.borrow().clone(),
            imp.stack.borrow().clone(),
            imp.empty.borrow().clone(),
        ) else {
            return;
        };

        while let Some(child) = list.first_child() {
            list.remove(&child);
        }

        let shown: Vec<&Hit> = hits.iter().take(LIMIT).collect();
        for hit in &shown {
            let title = gtk::Label::builder()
                .label(&hit.title)
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

            if !hit.detail.is_empty() {
                let detail = gtk::Label::builder()
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .build();
                detail.set_markup(&marked_up(&hit.detail, hit.highlight));
                detail.add_css_class("note-row-excerpt");
                detail.add_css_class("dimmed");
                row_box.append(&detail);
            }

            list.append(&gtk::ListBoxRow::builder().child(&row_box).build());
        }

        imp.ids
            .replace(shown.iter().map(|hit| hit.id.clone()).collect());

        if let Some(first) = list.row_at_index(0) {
            list.select_row(Some(&first));
        }

        let query = self.query();
        empty.set_description(Some(&if query.is_empty() {
            "Type to search.".to_string()
        } else {
            format!("Nothing matches “{query}”.")
        }));
        stack.set_visible_child_name(if shown.is_empty() { "empty" } else { "list" });
    }

    fn move_selection(&self, delta: i32) {
        let Some(list) = self.imp().list.borrow().clone() else {
            return;
        };
        let count = self.imp().ids.borrow().len() as i32;
        if count == 0 {
            return;
        }
        let current = list.selected_row().map(|row| row.index()).unwrap_or(0);
        let next = (current + delta).clamp(0, count - 1);
        let Some(row) = list.row_at_index(next) else {
            return;
        };
        list.select_row(Some(&row));
        self.scroll_into_view(&row);
    }

    /// Scroll a row into view without moving the focus.
    ///
    /// `grab_focus` would scroll too, and would take the caret out of the
    /// entry — which is where the typing has to stay.
    fn scroll_into_view(&self, row: &gtk::ListBoxRow) {
        let Some(scroller) = self.imp().scroller.borrow().clone() else {
            return;
        };
        let Some(list) = self.imp().list.borrow().clone() else {
            return;
        };
        let adjustment = scroller.vadjustment();

        // Measured against the list, which is the scroller's child, so the
        // row's y is already the scroll offset.
        let Some(bounds) = row.compute_bounds(&list) else {
            return;
        };
        let top = bounds.y() as f64;
        let bottom = top + bounds.height() as f64;

        if top < adjustment.value() {
            adjustment.set_value(top);
        } else if bottom > adjustment.value() + adjustment.page_size() {
            adjustment.set_value(bottom - adjustment.page_size());
        }
    }

    fn choose_selected(&self) {
        let Some(list) = self.imp().list.borrow().clone() else {
            return;
        };
        let Some(index) = list.selected_row().map(|row| row.index()) else {
            return;
        };
        let id = self.imp().ids.borrow().get(index.max(0) as usize).cloned();
        if let Some(id) = id {
            self.emit_by_name::<()>("chosen", &[&id]);
            self.close();
        }
    }
}

/// `detail` as Pango markup, with the matched range in bold.
///
/// Escaped first: a note containing `<b>` would otherwise turn the rest of the
/// list bold, and one containing an unbalanced `<` would fail to render at all.
fn marked_up(detail: &str, highlight: Option<(usize, usize)>) -> String {
    let Some((start, end)) = highlight else {
        return glib::markup_escape_text(detail).to_string();
    };

    let chars: Vec<char> = detail.chars().collect();
    let start = start.min(chars.len());
    let end = end.clamp(start, chars.len());

    let piece = |range: std::ops::Range<usize>| -> String {
        glib::markup_escape_text(&chars[range].iter().collect::<String>()).to_string()
    };

    format!(
        "{}<b>{}</b>{}",
        piece(0..start),
        piece(start..end),
        piece(end..chars.len())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_highlight_is_wrapped_in_bold() {
        assert_eq!(
            marked_up("Moves are destructive", Some((10, 21))),
            "Moves are <b>destructive</b>"
        );
    }

    #[test]
    fn markup_in_a_note_cannot_escape_into_the_label() {
        // A note containing "<b>" would otherwise embolden the rest of the list.
        assert_eq!(
            marked_up("a <b>bold</b> claim", None),
            "a &lt;b&gt;bold&lt;/b&gt; claim"
        );
        assert_eq!(
            marked_up("5 < 6 & 7 > 2", Some((0, 1))),
            "<b>5</b> &lt; 6 &amp; 7 &gt; 2"
        );
    }

    #[test]
    fn a_highlight_is_measured_in_characters() {
        assert_eq!(marked_up("🎉 wörld", Some((2, 7))), "🎉 <b>wörld</b>");
    }

    #[test]
    fn an_out_of_range_highlight_does_not_panic() {
        assert_eq!(marked_up("short", Some((3, 99))), "sho<b>rt</b>");
        assert_eq!(marked_up("short", Some((99, 99))), "short<b></b>");
    }
}
