//! The list of notes.
//!
//! A `ListView` over a `gio::ListStore` of [`NoteObject`], with a
//! `SignalListItemFactory`. Rows recycle, so the factory's `bind` must set
//! every field it ever sets — a row arriving with the previous note's excerpt
//! still on it is the classic bug here.
//!
//! The sidebar emits which note was chosen and never opens one itself. The
//! row menu is the same `win.` actions as the main menu, which act on the open
//! note — so a secondary click chooses the note first and the menu then has
//! one meaning, whichever way it was reached.

use std::cell::RefCell;
use std::sync::OnceLock;

use gtk::glib::subclass::Signal;
use gtk::glib::{self, clone};
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::model::note::NoteId;
use crate::ui::NoteObject;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Sidebar {
        pub store: RefCell<Option<gtk::gio::ListStore>>,
        pub selection: RefCell<Option<gtk::SingleSelection>>,
        pub list: RefCell<Option<gtk::ListView>>,
        pub empty: RefCell<Option<adw::StatusPage>>,
        pub stack: RefCell<Option<gtk::Stack>>,
        pub menu: RefCell<Option<gtk::PopoverMenu>>,
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
                vec![Signal::builder("note-chosen")
                    .param_types([String::static_type()])
                    .build()]
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

impl Sidebar {
    pub fn new() -> Self {
        glib::Object::new()
    }

    fn build(&self) {
        let store = gtk::gio::ListStore::new::<NoteObject>();
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

                let row = gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .spacing(2)
                    .margin_top(8)
                    .margin_bottom(8)
                    .margin_start(6)
                    .margin_end(6)
                    .build();
                row.append(&heading);
                row.append(&excerpt);

                // The gesture is set up once per recycled row and reads the
                // note off the `ListItem` when it fires, so it follows
                // whichever note the row currently holds.
                let secondary = gtk::GestureClick::builder()
                    .button(gtk::gdk::BUTTON_SECONDARY)
                    .build();
                secondary.connect_pressed(clone!(
                    #[weak]
                    sidebar,
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
                    #[weak]
                    sidebar,
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

                item.set_child(Some(&row));
            }
        ));

        factory.connect_bind(|_, item| {
            let item = item
                .downcast_ref::<gtk::ListItem>()
                .expect("list items are ListItems");
            let Some(note) = item.item().and_downcast::<NoteObject>() else {
                return;
            };
            let Some(row) = item.child().and_downcast::<gtk::Box>() else {
                return;
            };
            let Some(heading) = row.first_child().and_downcast::<gtk::Box>() else {
                return;
            };
            let Some(title) = heading.first_child().and_downcast::<gtk::Label>() else {
                return;
            };
            let Some(folder) = title.next_sibling().and_downcast::<gtk::Label>() else {
                return;
            };
            let Some(excerpt) = heading.next_sibling().and_downcast::<gtk::Label>() else {
                return;
            };

            // Rows recycle, so every field is set on every bind — including the
            // empty cases, or a row shows the previous note's folder.
            title.set_text(&note.title());
            folder.set_text(&note.folder());
            folder.set_visible(!note.folder().is_empty());
            let text = note.excerpt();
            excerpt.set_text(&text);
            excerpt.set_visible(!text.is_empty());
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
                let Some(note) = list
                    .model()
                    .and_then(|model| model.item(position))
                    .and_downcast::<NoteObject>()
                else {
                    return;
                };
                sidebar.emit_by_name::<()>("note-chosen", &[&note.id()]);
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

        // No button and no icon here. The + directly above this is one
        // affordance for writing a note and the content pane's button is
        // another; a third, in a 300px column, is clutter rather than help.
        let empty = adw::StatusPage::builder()
            .title("No Notes")
            .description("Notes appear here as you write them.")
            .build();
        empty.add_css_class("compact");

        let stack = gtk::Stack::new();
        stack.add_named(&scroller, Some("list"));
        stack.add_named(&empty, Some("empty"));
        stack.set_parent(self);
        self.set_vexpand(true);

        // The actions live on the window, and this popover is inside it, so
        // `win.` resolves without the sidebar knowing what they do.
        let model = gtk::gio::Menu::new();
        model.append(Some("_Rename…"), Some("win.rename-note"));
        model.append(Some("_Delete…"), Some("win.delete-note"));
        let menu = gtk::PopoverMenu::from_model(Some(&model));
        menu.set_has_arrow(false);
        menu.set_halign(gtk::Align::Start);
        menu.set_parent(self);

        self.imp().store.replace(Some(store));
        self.imp().selection.replace(Some(selection));
        self.imp().list.replace(Some(list));
        self.imp().empty.replace(Some(empty));
        self.imp().stack.replace(Some(stack));
        self.imp().menu.replace(Some(menu));
    }

    /// Choose the row's note, then show the menu at the pointer.
    fn open_row_menu(&self, item: &gtk::ListItem, row: &gtk::Widget, x: f64, y: f64) {
        let Some(note) = item.item().and_downcast::<NoteObject>() else {
            return;
        };
        self.emit_by_name::<()>("note-chosen", &[&note.id()]);
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
    /// already the chosen note — focus follows selection here.
    fn open_focused_menu(&self, row: &gtk::Widget) {
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

    fn popup_menu(&self, at: gtk::gdk::Rectangle) {
        let Some(menu) = self.imp().menu.borrow().clone() else {
            return;
        };
        menu.set_pointing_to(Some(&at));
        menu.popup();
    }

    /// Replace the whole list.
    ///
    /// Rebuilding rather than diffing: a personal vault is thousands of rows at
    /// most, `ListView` only realises the visible ones, and a diff that is
    /// subtly wrong shows the user a note that is not there.
    pub fn set_notes(&self, notes: &[(NoteId, String)]) {
        let imp = self.imp();
        let Some(store) = imp.store.borrow().clone() else {
            return;
        };

        let objects: Vec<NoteObject> = notes
            .iter()
            .map(|(id, excerpt)| NoteObject::new(id, excerpt))
            .collect();

        imp.selecting.set(true);
        store.remove_all();
        store.extend_from_slice(&objects);
        imp.selecting.set(false);

        if let Some(stack) = imp.stack.borrow().as_ref() {
            stack.set_visible_child_name(if objects.is_empty() { "empty" } else { "list" });
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
                        .and_downcast::<NoteObject>()
                        .is_some_and(|note| note.id() == id.as_str())
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
}
