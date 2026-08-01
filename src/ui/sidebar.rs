//! The folder tree of notes.
//!
//! A `ListView` over a `gio::ListStore` of [`RowObject`], with a
//! `SignalListItemFactory`. The tree is flattened into rows by
//! [`crate::model::tree::rows`] before it gets here, so this widget never walks
//! a hierarchy — it draws a list where each row knows its own depth. That is
//! what keeps the expansion logic testable without a display, and what lets the
//! same widget show flat search results by being handed different rows.
//!
//! Rows recycle, so the factory's `bind` must set every field it ever sets —
//! a row arriving with the previous note's excerpt still on it, or a note
//! wearing the chevron of the folder that was there before, is the classic bug
//! here.
//!
//! The sidebar emits what the user did and changes nothing itself: no file is
//! moved, no folder is created, no note is opened. A drag that ends on a folder
//! is a `moved` signal, and the application decides whether that is a rename it
//! is willing to do.

use std::cell::RefCell;
use std::sync::OnceLock;

use gtk::glib::subclass::Signal;
use gtk::glib::{self, clone};
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::model::note::NoteId;
use crate::model::tree::Row;
use crate::ui::RowObject;

/// How far one level of nesting indents, in pixels.
const INDENT: i32 = 16;

/// What is being dragged, as it travels on the clipboard.
///
/// One string with a prefix rather than two content types: a drop target that
/// accepts both would have to ask which it got anyway, and the prefix is
/// readable in a debugger.
const NOTE_PREFIX: &str = "brain-note:";
const FOLDER_PREFIX: &str = "brain-folder:";

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Sidebar {
        pub store: RefCell<Option<gtk::gio::ListStore>>,
        pub selection: RefCell<Option<gtk::SingleSelection>>,
        pub list: RefCell<Option<gtk::ListView>>,
        pub stack: RefCell<Option<gtk::Stack>>,
        pub menu: RefCell<Option<gtk::PopoverMenu>>,
        /// The strip under the list that means the vault root, and the label on
        /// it that only says so while something is being dragged.
        pub root_strip: RefCell<Option<gtk::Box>>,
        pub root_label: RefCell<Option<gtk::Label>>,
        /// The note menu, kept so the popover can be put back to it after a
        /// folder row has borrowed the popover for its own.
        pub note_menu: RefCell<Option<gtk::gio::Menu>>,
        /// Set while the selection is being restored in code, so the handler
        /// does not report it as the user choosing a note and reload the
        /// editor underneath them.
        pub selecting: std::cell::Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Sidebar {
        const NAME: &'static str = "BrainSidebar";
        type Type = super::Sidebar;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for Sidebar {
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
                vec![
                    Signal::builder("note-chosen")
                        .param_types([String::static_type()])
                        .build(),
                    // A folder was activated: open or close it, and treat it as
                    // where the next new note goes.
                    Signal::builder("folder-activated")
                        .param_types([String::static_type()])
                        .build(),
                    // A note or folder was dragged onto a folder. The payload
                    // carries which, and the second argument is the destination
                    // folder — empty for the vault root.
                    Signal::builder("moved")
                        .param_types([String::static_type(), String::static_type()])
                        .build(),
                ]
            })
        }
    }

    impl WidgetImpl for Sidebar {}
}

glib::wrapper! {
    pub struct Sidebar(ObjectSubclass<imp::Sidebar>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Sidebar {
    fn default() -> Self {
        Self::new()
    }
}

/// The widgets of one recycled row, found once rather than at every access.
struct RowWidgets {
    row: gtk::Box,
    chevron: gtk::Image,
    title: gtk::Label,
    folder: gtk::Label,
    excerpt: gtk::Label,
    count: gtk::Label,
}

impl RowWidgets {
    fn build() -> Self {
        let chevron = gtk::Image::from_icon_name("pan-end-symbolic");
        chevron.set_visible(false);

        let title = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        title.add_css_class("note-row-title");

        let folder = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .build();
        folder.add_css_class("note-row-folder");
        folder.add_css_class("dimmed");

        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        heading.append(&title);
        heading.append(&folder);

        let excerpt = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        excerpt.add_css_class("note-row-excerpt");
        excerpt.add_css_class("dimmed");

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .build();
        content.append(&heading);
        content.append(&excerpt);

        let count = gtk::Label::new(None);
        count.add_css_class("dimmed");
        count.add_css_class("numeric");
        count.set_visible(false);

        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(6)
            .margin_end(6)
            .build();
        row.append(&chevron);
        row.append(&content);
        row.append(&count);

        Self {
            row,
            chevron,
            title,
            folder,
            excerpt,
            count,
        }
    }

    /// Find the widgets again on a recycled row. The order they were appended
    /// in is the contract, and it is only written down here.
    fn of(row: &gtk::Box) -> Option<Self> {
        let chevron = row.first_child()?.downcast::<gtk::Image>().ok()?;
        let content = chevron.next_sibling()?.downcast::<gtk::Box>().ok()?;
        let count = content.next_sibling()?.downcast::<gtk::Label>().ok()?;
        let heading = content.first_child()?.downcast::<gtk::Box>().ok()?;
        let excerpt = heading.next_sibling()?.downcast::<gtk::Label>().ok()?;
        let title = heading.first_child()?.downcast::<gtk::Label>().ok()?;
        let folder = title.next_sibling()?.downcast::<gtk::Label>().ok()?;
        Some(Self {
            row: row.clone(),
            chevron,
            title,
            folder,
            excerpt,
            count,
        })
    }

    fn bind(&self, item: &RowObject) {
        let depth = item.depth() as i32;
        self.row.set_margin_start(6 + depth * INDENT);
        self.title.set_text(&item.title());

        let folder = item.folder();
        self.folder.set_text(&folder);
        self.folder.set_visible(!folder.is_empty());

        let excerpt = item.excerpt();
        self.excerpt.set_text(&excerpt);
        self.excerpt.set_visible(!excerpt.is_empty());

        if item.is_folder() {
            self.chevron.set_visible(true);
            self.chevron.set_icon_name(Some(if item.expanded() {
                "pan-down-symbolic"
            } else {
                "pan-end-symbolic"
            }));
            self.count.set_text(&item.count().to_string());
            self.count.set_visible(true);
            self.row.add_css_class("folder-row");
        } else {
            self.chevron.set_visible(false);
            self.count.set_visible(false);
            self.row.remove_css_class("folder-row");
        }
    }
}

impl Sidebar {
    pub fn new() -> Self {
        glib::Object::new()
    }

    fn build(&self) {
        let store = gtk::gio::ListStore::new::<RowObject>();
        let selection = gtk::SingleSelection::builder()
            .model(&store)
            .autoselect(false)
            .can_unselect(true)
            .build();

        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(clone!(
            #[weak(rename_to = sidebar)]
            self,
            move |_, item| {
                let item = item
                    .downcast_ref::<gtk::ListItem>()
                    .expect("list items are ListItems");
                let widgets = RowWidgets::build();

                // Every controller is set up once per recycled row and reads
                // the row off the `ListItem` when it fires, so it follows
                // whichever note or folder the row currently holds.
                sidebar.add_row_menu(item, &widgets.row);
                sidebar.add_drag_source(item, &widgets.row);
                sidebar.add_drop_target(item, &widgets.row);

                item.set_child(Some(&widgets.row));
            }
        ));

        factory.connect_bind(|_, item| {
            let item = item
                .downcast_ref::<gtk::ListItem>()
                .expect("list items are ListItems");
            let (Some(object), Some(row)) = (
                item.item().and_downcast::<RowObject>(),
                item.child().and_downcast::<gtk::Box>(),
            ) else {
                return;
            };
            if let Some(widgets) = RowWidgets::of(&row) {
                widgets.bind(&object);
            }
        });

        let list = gtk::ListView::builder()
            .model(&selection)
            .factory(&factory)
            .single_click_activate(true)
            .build();
        list.add_css_class("navigation-sidebar");

        list.connect_activate(clone!(
            #[weak(rename_to = sidebar)]
            self,
            move |list, position| {
                let Some(object) = list
                    .model()
                    .and_then(|model| model.item(position))
                    .and_downcast::<RowObject>()
                else {
                    return;
                };
                sidebar.activate_row(&object);
            }
        ));

        // Shift+F10 and the Menu key reach the same menu from the keyboard,
        // for the row that has focus.
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(clone!(
            #[weak(rename_to = sidebar)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |controller, key, _, state| {
                let wanted = key == gtk::gdk::Key::Menu
                    || (key == gtk::gdk::Key::F10
                        && state.contains(gtk::gdk::ModifierType::SHIFT_MASK));
                if !wanted {
                    return glib::Propagation::Proceed;
                }
                let Some(focused) = controller
                    .widget()
                    .and_then(|list| list.root().and_then(|root| root.focus()))
                else {
                    return glib::Propagation::Proceed;
                };
                sidebar.open_focused_menu(&focused);
                glib::Propagation::Stop
            }
        ));
        list.add_controller(keys);

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();

        // Dropping in the space below the rows means the vault root. The row
        // targets take everything that lands on a row, so this only ever sees
        // a drop that missed.
        scroller.add_controller(self.root_drop_target(None));

        // …but a tree taller than the pane leaves no such space, and then there
        // is no way to drag a note back out to the root at all. This strip is
        // pinned below the scroller so the root is always a target, whatever
        // the list is doing. It keeps its height when idle rather than
        // appearing mid-drag, since a strip that grows under the pointer moves
        // the rows the drag was aimed at.
        let root_label = gtk::Label::new(None);
        root_label.add_css_class("dimmed");
        root_label.add_css_class("caption");
        let root_strip = gtk::Box::builder()
            .halign(gtk::Align::Fill)
            .margin_start(6)
            .margin_end(6)
            .margin_bottom(6)
            .build();
        root_strip.add_css_class("root-strip");
        root_strip.append(&root_label);
        root_label.set_hexpand(true);
        root_strip.add_controller(self.root_drop_target(Some(&root_strip)));

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&scroller);
        column.append(&root_strip);

        // No button and no icon here. The + directly above this is one
        // affordance for writing a note and the content pane's button is
        // another; a third, in a 300px column, is clutter rather than help.
        let empty = adw::StatusPage::builder()
            .title("No Notes")
            .description("Notes appear here as you write them.")
            .build();
        empty.add_css_class("compact");

        let no_results = adw::StatusPage::builder()
            .icon_name("system-search-symbolic")
            .title("No Matches")
            .description("No note has that in its title or its text.")
            .build();
        no_results.add_css_class("compact");

        let stack = gtk::Stack::new();
        stack.add_named(&column, Some("list"));
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&no_results, Some("no-results"));
        stack.set_parent(self);
        self.set_vexpand(true);

        // The actions live on the window, and this popover is inside it, so
        // `win.` resolves without the sidebar knowing what they do.
        let note_menu = gtk::gio::Menu::new();
        note_menu.append(Some("_Rename…"), Some("win.rename-note"));
        note_menu.append(Some("_Delete…"), Some("win.delete-note"));
        let menu = gtk::PopoverMenu::from_model(Some(&note_menu));
        menu.set_has_arrow(false);
        menu.set_halign(gtk::Align::Start);
        menu.set_parent(self);

        self.imp().store.replace(Some(store));
        self.imp().selection.replace(Some(selection));
        self.imp().list.replace(Some(list));
        self.imp().stack.replace(Some(stack));
        self.imp().menu.replace(Some(menu));
        self.imp().note_menu.replace(Some(note_menu));
        self.imp().root_strip.replace(Some(root_strip));
        self.imp().root_label.replace(Some(root_label));
    }

    /// A target that moves whatever is dropped on it to the vault root.
    fn root_drop_target(&self, highlight: Option<&gtk::Box>) -> gtk::DropTarget {
        let target = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
        if let Some(strip) = highlight {
            target.connect_enter(clone!(
                #[weak]
                strip,
                #[upgrade_or]
                gtk::gdk::DragAction::empty(),
                move |_, _, _| {
                    strip.add_css_class("drop-into");
                    gtk::gdk::DragAction::MOVE
                }
            ));
            target.connect_leave(clone!(
                #[weak]
                strip,
                move |_| strip.remove_css_class("drop-into")
            ));
        }
        target.connect_drop(clone!(
            #[weak(rename_to = sidebar)]
            self,
            #[upgrade_or]
            false,
            move |_, value, _, _| {
                let Ok(payload) = value.get::<String>() else {
                    return false;
                };
                sidebar.emit_by_name::<()>("moved", &[&payload, &String::new()]);
                true
            }
        ));
        target
    }

    /// Name the root strip while a drag is in the air, and only then: an
    /// instruction standing there permanently is noise in a 300px column, but
    /// an unlabelled strip is not a target anyone would find.
    fn set_dragging(&self, dragging: bool) {
        let imp = self.imp();
        if let Some(label) = imp.root_label.borrow().as_ref() {
            label.set_text(if dragging { "Move to Vault Root" } else { "" });
        }
        if let Some(strip) = imp.root_strip.borrow().as_ref() {
            if dragging {
                strip.add_css_class("root-drop");
            } else {
                strip.remove_css_class("root-drop");
                strip.remove_css_class("drop-into");
            }
        }
    }

    /// Report what was activated. A folder opens or closes; a note is opened.
    fn activate_row(&self, object: &RowObject) {
        if object.is_folder() {
            self.emit_by_name::<()>("folder-activated", &[&object.id()]);
        } else {
            self.emit_by_name::<()>("note-chosen", &[&object.id()]);
        }
    }

    // ---- dragging ----

    fn add_drag_source(&self, item: &gtk::ListItem, row: &gtk::Box) {
        let source = gtk::DragSource::builder()
            .actions(gtk::gdk::DragAction::MOVE)
            .build();
        source.connect_prepare(clone!(
            #[weak]
            item,
            #[upgrade_or]
            None,
            move |_, _, _| {
                let object = item.item().and_downcast::<RowObject>()?;
                let payload = payload_of(&object);
                Some(gtk::gdk::ContentProvider::for_value(&payload.to_value()))
            }
        ));
        // The row itself is the drag icon, so what is being moved is never in
        // doubt while it is in the air.
        source.connect_drag_begin(clone!(
            #[weak(rename_to = sidebar)]
            self,
            #[weak]
            row,
            move |source, _| {
                let paintable = gtk::WidgetPaintable::new(Some(&row));
                source.set_icon(Some(&paintable), 0, 0);
                sidebar.set_dragging(true);
            }
        ));
        // `drag-end` fires whether the drop landed or was cancelled, so the
        // strip cannot be left labelled with nothing in the air.
        source.connect_drag_end(clone!(
            #[weak(rename_to = sidebar)]
            self,
            move |_, _, _| sidebar.set_dragging(false)
        ));
        row.add_controller(source);
    }

    fn add_drop_target(&self, item: &gtk::ListItem, row: &gtk::Box) {
        let target = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);

        target.connect_enter(clone!(
            #[weak]
            row,
            #[upgrade_or]
            gtk::gdk::DragAction::empty(),
            move |_, _, _| {
                row.add_css_class("drop-into");
                gtk::gdk::DragAction::MOVE
            }
        ));
        target.connect_leave(clone!(
            #[weak]
            row,
            move |_| row.remove_css_class("drop-into")
        ));

        target.connect_drop(clone!(
            #[weak(rename_to = sidebar)]
            self,
            #[weak]
            item,
            #[weak]
            row,
            #[upgrade_or]
            false,
            move |_, value, _, _| {
                row.remove_css_class("drop-into");
                let (Ok(payload), Some(object)) = (
                    value.get::<String>(),
                    item.item().and_downcast::<RowObject>(),
                ) else {
                    return false;
                };
                // Dropping on a note means the folder that note is in, which is
                // what aiming at a list of siblings looks like it should do.
                let destination = if object.is_folder() {
                    object.id()
                } else {
                    object.note_id().folder().unwrap_or("").to_string()
                };
                if payload == payload_of(&object) {
                    return false; // dropped on itself
                }
                sidebar.emit_by_name::<()>("moved", &[&payload, &destination]);
                true
            }
        ));

        row.add_controller(target);
    }

    // ---- menus ----

    fn add_row_menu(&self, item: &gtk::ListItem, row: &gtk::Box) {
        let secondary = gtk::GestureClick::builder()
            .button(gtk::gdk::BUTTON_SECONDARY)
            .build();
        secondary.connect_pressed(clone!(
            #[weak(rename_to = sidebar)]
            self,
            #[weak]
            item,
            move |gesture, _, x, y| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                let Some(widget) = gesture.widget() else {
                    return;
                };
                sidebar.open_row_menu(&item, &widget, x, y);
            }
        ));
        row.add_controller(secondary);

        // Touch: the same menu, from a press and hold.
        let hold = gtk::GestureLongPress::builder().touch_only(true).build();
        hold.connect_pressed(clone!(
            #[weak(rename_to = sidebar)]
            self,
            #[weak]
            item,
            move |gesture, x, y| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                let Some(widget) = gesture.widget() else {
                    return;
                };
                sidebar.open_row_menu(&item, &widget, x, y);
            }
        ));
        row.add_controller(hold);
    }

    /// Choose the row, then show its menu at the pointer.
    fn open_row_menu(&self, item: &gtk::ListItem, row: &gtk::Widget, x: f64, y: f64) {
        let Some(object) = item.item().and_downcast::<RowObject>() else {
            return;
        };
        // A secondary click chooses the row first, so the menu has one meaning
        // whichever way it was reached. A folder is not opened by it, though —
        // asking for a folder's menu is not asking to collapse it.
        if !object.is_folder() {
            self.emit_by_name::<()>("note-chosen", &[&object.id()]);
        }
        self.set_menu_for(&object);

        let Some(point) = row.compute_point(self, &gtk::graphene::Point::new(x as f32, y as f32))
        else {
            return;
        };
        self.popup_menu(gtk::gdk::Rectangle::new(
            point.x() as i32,
            point.y() as i32,
            1,
            1,
        ));
    }

    /// Show the menu against a focused row, for the keyboard path. The row is
    /// already the chosen one — focus follows selection here.
    fn open_focused_menu(&self, row: &gtk::Widget) {
        if let Some(object) = self.selected_object() {
            self.set_menu_for(&object);
        }
        let Some(bounds) = row.compute_bounds(self) else {
            return;
        };
        self.popup_menu(gtk::gdk::Rectangle::new(
            bounds.x() as i32,
            bounds.y() as i32,
            bounds.width() as i32,
            bounds.height() as i32,
        ));
    }

    /// Put the right menu on the popover.
    ///
    /// A folder's items carry the folder as their action target, so the window
    /// acts on the folder that was clicked rather than on whatever happens to
    /// be selected — a menu that acts on something else is worse than no menu.
    fn set_menu_for(&self, object: &RowObject) {
        let Some(menu) = self.imp().menu.borrow().clone() else {
            return;
        };
        if !object.is_folder() {
            if let Some(model) = self.imp().note_menu.borrow().as_ref() {
                menu.set_menu_model(Some(model));
            }
            return;
        }

        let path = object.id().to_variant();
        let model = gtk::gio::Menu::new();
        let create = gtk::gio::Menu::new();
        for (label, action) in [
            ("_New Note Here", "win.new-note-in"),
            ("New _Folder Here…", "win.new-folder-in"),
        ] {
            let item = gtk::gio::MenuItem::new(Some(label), None);
            item.set_action_and_target_value(Some(action), Some(&path));
            create.append_item(&item);
        }
        model.append_section(None, &create);

        let edit = gtk::gio::Menu::new();
        for (label, action) in [
            ("_Rename Folder…", "win.rename-folder"),
            ("_Delete Folder…", "win.delete-folder"),
        ] {
            let item = gtk::gio::MenuItem::new(Some(label), None);
            item.set_action_and_target_value(Some(action), Some(&path));
            edit.append_item(&item);
        }
        model.append_section(None, &edit);

        menu.set_menu_model(Some(&model));
    }

    fn popup_menu(&self, at: gtk::gdk::Rectangle) {
        let Some(menu) = self.imp().menu.borrow().clone() else {
            return;
        };
        menu.set_pointing_to(Some(&at));
        menu.popup();
    }

    fn selected_object(&self) -> Option<RowObject> {
        let selection = self.imp().selection.borrow().clone()?;
        selection.selected_item().and_downcast::<RowObject>()
    }

    // ---- what the window puts in it ----

    /// Replace the whole list with a tree.
    ///
    /// Rebuilding rather than diffing: a personal vault is thousands of rows at
    /// most, `ListView` only realises the visible ones, and a diff that is
    /// subtly wrong shows the user a note that is not there.
    pub fn set_rows(&self, rows: &[Row]) {
        let objects: Vec<RowObject> = rows.iter().map(RowObject::from_row).collect();
        self.replace(&objects, "empty");
    }

    /// Replace the whole list with search results: no tree, no indent, and the
    /// folder spelled out on every row.
    pub fn set_results(&self, results: &[(NoteId, String)]) {
        let objects: Vec<RowObject> = results
            .iter()
            .map(|(id, excerpt)| RowObject::result(id, excerpt))
            .collect();
        self.replace(&objects, "no-results");
    }

    fn replace(&self, objects: &[RowObject], when_empty: &str) {
        let imp = self.imp();
        let Some(store) = imp.store.borrow().clone() else {
            return;
        };

        imp.selecting.set(true);
        store.remove_all();
        store.extend_from_slice(objects);
        imp.selecting.set(false);

        if let Some(stack) = imp.stack.borrow().as_ref() {
            stack.set_visible_child_name(if objects.is_empty() {
                when_empty
            } else {
                "list"
            });
        }
    }

    /// Highlight a note without emitting `note-chosen`.
    pub fn select(&self, id: Option<&NoteId>) {
        let imp = self.imp();
        let (Some(store), Some(selection)) =
            (imp.store.borrow().clone(), imp.selection.borrow().clone())
        else {
            return;
        };

        imp.selecting.set(true);
        match id {
            Some(id) => {
                let found = (0..store.n_items()).find(|index| {
                    store
                        .item(*index)
                        .and_downcast::<RowObject>()
                        .is_some_and(|row| !row.is_folder() && row.id() == id.as_str())
                });
                match found {
                    Some(index) => selection.set_selected(index),
                    None => selection.set_selected(gtk::INVALID_LIST_POSITION),
                }
            }
            None => selection.set_selected(gtk::INVALID_LIST_POSITION),
        }
        imp.selecting.set(false);
    }

    /// The rows as they stand, for tests.
    pub fn row_ids_for_test(&self) -> Vec<String> {
        let Some(store) = self.imp().store.borrow().clone() else {
            return Vec::new();
        };
        (0..store.n_items())
            .filter_map(|index| store.item(index).and_downcast::<RowObject>())
            .map(|row| {
                if row.is_folder() {
                    format!("{}/", row.id())
                } else {
                    row.id()
                }
            })
            .collect()
    }

    /// What the root strip says, and whether it is on screen at all, for tests.
    pub fn root_strip_for_test(&self, dragging: bool) -> Option<(bool, String)> {
        self.set_dragging(dragging);
        let strip = self.imp().root_strip.borrow().clone()?;
        let label = self.imp().root_label.borrow().clone()?;
        Some((strip.is_visible(), label.text().to_string()))
    }

    /// Drive a drop the way the drag would, for tests.
    pub fn drop_for_test(&self, payload: &str, destination: &str) {
        self.emit_by_name::<()>("moved", &[&payload.to_string(), &destination.to_string()]);
    }
}

fn payload_of(object: &RowObject) -> String {
    if object.is_folder() {
        format!("{FOLDER_PREFIX}{}", object.id())
    } else {
        format!("{NOTE_PREFIX}{}", object.id())
    }
}

/// What a drop payload refers to, or `None` if it came from outside Brain.
pub fn dragged(payload: &str) -> Option<Dragged> {
    if let Some(id) = payload.strip_prefix(NOTE_PREFIX) {
        return Some(Dragged::Note(NoteId::from_relative(id)));
    }
    payload
        .strip_prefix(FOLDER_PREFIX)
        .map(|path| Dragged::Folder(path.to_string()))
}

/// What was dragged onto a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dragged {
    Note(NoteId),
    Folder(String),
}
