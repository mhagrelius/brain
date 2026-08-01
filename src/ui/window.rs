//! The main window: sidebar, editor, and the dialogs that act on a note.
//!
//! The window owns no note data. It asks [`BrainApplication`] for the list, it
//! tells the application what the user did, and the application tells it what
//! to show. Every dialog here ends in an application call, never a file write.

use std::cell::RefCell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib::{self, clone};

use crate::model::note::NoteId;
use crate::ui::{
    BacklinksPanel, BrainApplication, DetailsPanel, Editor, Mode, Palette, Sidebar, TagTree,
};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct BrainWindow {
        pub split: RefCell<Option<adw::OverlaySplitView>>,
        pub sidebar: RefCell<Option<Sidebar>>,
        pub editor: RefCell<Option<Editor>>,
        pub stack: RefCell<Option<gtk::Stack>>,
        pub toasts: RefCell<Option<adw::ToastOverlay>>,
        pub title: RefCell<Option<adw::WindowTitle>>,
        pub status: RefCell<Option<gtk::Label>>,
        pub backlinks: RefCell<Option<BacklinksPanel>>,
        pub backlinks_title: RefCell<Option<adw::WindowTitle>>,
        pub details: RefCell<Option<DetailsPanel>>,
        pub detail_stack: RefCell<Option<adw::ViewStack>>,
        pub backlink_count: std::cell::Cell<usize>,
        pub actions: RefCell<Option<gtk::gio::SimpleActionGroup>>,
        pub tags: RefCell<Option<TagTree>>,
        pub sidebar_stack: RefCell<Option<adw::ViewStack>>,
        pub sidebar_title: RefCell<Option<adw::WindowTitle>>,
        pub clear_filter: RefCell<Option<gtk::Button>>,
        /// Built once and reused, so reopening keeps the last query.
        pub palette: RefCell<Option<Palette>>,
        pub detail: RefCell<Option<adw::OverlaySplitView>>,
        pub reading_toggle: RefCell<Option<gtk::ToggleButton>>,
        /// Shown when notes are not reaching disk.
        pub banner: RefCell<Option<adw::Banner>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BrainWindow {
        const NAME: &'static str = "BrainWindow";
        type Type = super::BrainWindow;
        type ParentType = adw::ApplicationWindow;
    }

    impl ObjectImpl for BrainWindow {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.build();
            obj.install_actions();
        }
    }

    impl WidgetImpl for BrainWindow {}

    impl WindowImpl for BrainWindow {
        fn close_request(&self) -> glib::Propagation {
            // Flush before the window goes: the tick may be up to two seconds
            // away and those two seconds are the user's typing.
            if let Some(app) = self.obj().brain_application() {
                app.flush_open_note();
                app.save_now();
                app.remember_window();
            }
            self.parent_close_request()
        }
    }

    impl ApplicationWindowImpl for BrainWindow {}
    impl AdwApplicationWindowImpl for BrainWindow {}
}

glib::wrapper! {
    pub struct BrainWindow(ObjectSubclass<imp::BrainWindow>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gtk::gio::ActionGroup, gtk::gio::ActionMap, gtk::Accessible,
                    gtk::Buildable, gtk::ConstraintTarget, gtk::Native, gtk::Root,
                    gtk::ShortcutManager;
}

impl BrainWindow {
    pub fn new(app: &BrainApplication) -> Self {
        glib::Object::builder().property("application", app).build()
    }

    fn brain_application(&self) -> Option<BrainApplication> {
        self.application().and_downcast::<BrainApplication>()
    }

    fn build(&self) {
        self.set_title(Some("Brain"));
        self.set_default_size(1000, 700);

        let sidebar = Sidebar::new();
        let editor = Editor::new();

        // ---- sidebar pane ----
        let new_button = gtk::Button::from_icon_name("list-add-symbolic");
        new_button.set_tooltip_text(Some("New Note (Ctrl+N)"));
        new_button.set_action_name(Some("win.new-note"));

        let tags = TagTree::new();

        // Two views of the same vault, switched in place. A tag is a way of
        // looking at notes, not a different kind of thing, so it belongs in
        // the same pane rather than a second one.
        let sidebar_stack = adw::ViewStack::new();
        sidebar_stack
            .add_titled_with_icon(&sidebar, Some("notes"), "Notes", "view-list-symbolic")
            .set_visible(true);
        sidebar_stack.add_titled_with_icon(&tags, Some("tags"), "Tags", "tag-symbolic");

        let switcher = adw::InlineViewSwitcher::builder()
            .stack(&sidebar_stack)
            .build();

        let sidebar_title = adw::WindowTitle::new("Notes", "");
        let sidebar_header = adw::HeaderBar::builder()
            .title_widget(&sidebar_title)
            .build();
        sidebar_header.pack_end(&new_button);

        // Only present while a tag is filtering the list, so the filter is
        // never a state you cannot see or leave.
        let clear_filter = gtk::Button::from_icon_name("edit-clear-symbolic");
        clear_filter.set_tooltip_text(Some("Show All Notes"));
        clear_filter.set_action_name(Some("win.clear-filter"));
        clear_filter.set_visible(false);
        sidebar_header.pack_start(&clear_filter);

        let switcher_bar = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .halign(gtk::Align::Center)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        switcher_bar.append(&switcher);

        let sidebar_pane = adw::ToolbarView::builder().content(&sidebar_stack).build();
        sidebar_pane.add_top_bar(&sidebar_header);
        sidebar_pane.add_top_bar(&switcher_bar);

        tags.connect_closure(
            "tag-chosen",
            false,
            glib::closure_local!(
                #[weak(rename_to = window)]
                self,
                move |_: TagTree, tag: String| {
                    if let Some(app) = window.brain_application() {
                        app.filter_by_tag(Some(&tag));
                    }
                }
            ),
        );

        // ---- content pane ----
        let title = adw::WindowTitle::new("Brain", "");
        let content_header = adw::HeaderBar::builder().title_widget(&title).build();

        let menu = gtk::gio::Menu::new();
        let note_section = gtk::gio::Menu::new();
        note_section.append(Some("_Rename…"), Some("win.rename-note"));
        note_section.append(Some("_Delete…"), Some("win.delete-note"));
        menu.append_section(None, &note_section);
        let vault_section = gtk::gio::Menu::new();
        vault_section.append(Some("_Change Vault…"), Some("win.choose-vault"));
        vault_section.append(Some("_Reload from Disk"), Some("win.reload"));
        vault_section.append(Some("_Unused Attachments…"), Some("win.unused-attachments"));
        menu.append_section(None, &vault_section);
        let help_section = gtk::gio::Menu::new();
        help_section.append(Some("_Keyboard Shortcuts"), Some("win.shortcuts"));
        help_section.append(Some("_About Brain"), Some("app.about"));
        menu.append_section(None, &help_section);

        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .tooltip_text("Main Menu")
            .build();
        content_header.pack_end(&menu_button);

        let status = gtk::Label::builder().xalign(1.0).build();
        status.add_css_class("status-line");
        status.add_css_class("dimmed");

        let banner = adw::Banner::new("");
        banner.set_revealed(false);

        let editor_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        editor_box.append(&banner);
        editor_box.append(&editor);
        editor_box.append(&status);

        // Shown when no note is open, which is a real state and not an error:
        // an empty vault, or one where nothing has been picked yet.
        let open_one = gtk::Button::builder()
            .label("New Note")
            .halign(gtk::Align::Center)
            .action_name("win.new-note")
            .build();
        open_one.add_css_class("pill");
        open_one.add_css_class("suggested-action");

        let placeholder = adw::StatusPage::builder()
            .icon_name("accessories-text-editor-symbolic")
            .title("No Note Open")
            .description("Your notes are Markdown files in a folder you own.")
            .child(&open_one)
            .build();

        let stack = gtk::Stack::new();
        stack.add_named(&editor_box, Some("editor"));
        stack.add_named(&placeholder, Some("placeholder"));
        stack.set_visible_child_name("placeholder");

        let content_pane = adw::ToolbarView::builder().content(&stack).build();
        content_pane.add_top_bar(&content_header);

        // Backlinks sit in a second split view packed to the END, the way
        // Planner carries its detail panel. Collapsed by default: the pane is
        // for when you go looking, not a permanent tax on the writing area.
        // The right pane carries two views of the open note, switched in
        // place the same way the left one switches Notes and Tags.
        let backlinks = BacklinksPanel::new();
        let details = DetailsPanel::new();

        let detail_stack = adw::ViewStack::new();
        detail_stack
            .add_titled_with_icon(
                &details,
                Some("details"),
                "Details",
                "document-edit-symbolic",
            )
            .set_visible(true);
        detail_stack.add_titled_with_icon(
            &backlinks,
            Some("backlinks"),
            "Backlinks",
            "insert-link-symbolic",
        );

        let backlinks_title = adw::WindowTitle::new("Details", "");
        // Default title buttons. Suppressing them here takes the window
        // controls away entirely while the pane is open, because this is the
        // rightmost bar and libadwaita puts them on whichever that is.
        let backlinks_header = adw::HeaderBar::builder()
            .title_widget(&backlinks_title)
            .build();

        let detail_switcher = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .halign(gtk::Align::Center)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        detail_switcher.append(
            &adw::InlineViewSwitcher::builder()
                .stack(&detail_stack)
                .build(),
        );

        let backlinks_pane = adw::ToolbarView::builder().content(&detail_stack).build();
        backlinks_pane.add_top_bar(&backlinks_header);
        backlinks_pane.add_top_bar(&detail_switcher);

        // The header title follows whichever view is showing, so the pane
        // always says what it is.
        detail_stack.connect_visible_child_name_notify(clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.update_detail_title()
        ));

        details.connect_closure(
            "format-requested",
            false,
            glib::closure_local!(
                #[weak(rename_to = window)]
                self,
                move |_: DetailsPanel, name: String| {
                    let Some(format) = DetailsPanel::format_from_name(&name) else {
                        return;
                    };
                    if let Some(editor) = window.imp().editor.borrow().as_ref() {
                        editor.apply_format(format);
                    }
                }
            ),
        );

        let detail = adw::OverlaySplitView::builder()
            .sidebar_position(gtk::PackType::End)
            .sidebar(&backlinks_pane)
            .content(&content_pane)
            .min_sidebar_width(260.0)
            .max_sidebar_width(360.0)
            .sidebar_width_fraction(0.28)
            .show_sidebar(false)
            .build();

        // Reading mode. Packed before the backlinks toggle so it sits nearest
        // the note it acts on.
        let reading_toggle = gtk::ToggleButton::builder()
            .icon_name("view-reveal-symbolic")
            .tooltip_text("Reading Mode (Ctrl+E)")
            .build();
        reading_toggle.connect_toggled(clone!(
            #[weak(rename_to = window)]
            self,
            move |toggle| window.apply_reading(toggle.is_active())
        ));
        content_header.pack_end(&reading_toggle);
        self.imp().reading_toggle.replace(Some(reading_toggle));

        let backlinks_toggle = gtk::ToggleButton::builder()
            .icon_name("sidebar-show-right-symbolic")
            .tooltip_text("Details and Backlinks (F10)")
            .build();
        backlinks_toggle
            .bind_property("active", &detail, "show-sidebar")
            .bidirectional()
            .sync_create()
            .build();
        content_header.pack_end(&backlinks_toggle);

        backlinks.connect_closure(
            "note-chosen",
            false,
            glib::closure_local!(
                #[weak(rename_to = window)]
                self,
                move |_: BacklinksPanel, id: String| {
                    if let Some(app) = window.brain_application() {
                        app.open_note(&NoteId::from_relative(id));
                    }
                }
            ),
        );

        // ---- split ----
        let split = adw::OverlaySplitView::builder()
            .sidebar(&sidebar_pane)
            .content(&detail)
            .min_sidebar_width(240.0)
            .max_sidebar_width(360.0)
            .sidebar_width_fraction(0.28)
            .build();

        // Active from the start, because `sync_create` copies the *toggle* into
        // the split view: left at its default the binding would hide the
        // sidebar the moment the window opened.
        let toggle = gtk::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text("Toggle Sidebar (F9)")
            .active(true)
            .build();
        toggle
            .bind_property("active", &split, "show-sidebar")
            .bidirectional()
            .sync_create()
            .build();
        content_header.pack_start(&toggle);

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&split));
        self.set_content(Some(&toasts));

        // One breakpoint: below it the sidebar becomes an overlay rather than
        // a column, which is the only sensible thing to do with 360sp.
        let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            675.0,
            adw::LengthUnit::Sp,
        ));
        breakpoint.add_setter(&split, "collapsed", Some(&true.to_value()));
        breakpoint.add_setter(&toggle, "active", Some(&false.to_value()));
        breakpoint.add_setter(&detail, "collapsed", Some(&true.to_value()));
        self.add_breakpoint(breakpoint);

        sidebar.connect_closure(
            "note-chosen",
            false,
            glib::closure_local!(
                #[weak(rename_to = window)]
                self,
                move |_: Sidebar, id: String| {
                    if let Some(app) = window.brain_application() {
                        app.open_note(&NoteId::from_relative(id));
                    }
                }
            ),
        );

        editor.connect_closure(
            "link-activated",
            false,
            glib::closure_local!(
                #[weak(rename_to = window)]
                self,
                move |_: Editor, target: String| {
                    if let Some(app) = window.brain_application() {
                        app.follow_link(&target);
                    }
                }
            ),
        );

        editor.connect_closure(
            "files-dropped",
            false,
            glib::closure_local!(
                #[weak(rename_to = window)]
                self,
                move |_: Editor, paths: Vec<String>| {
                    if let Some(app) = window.brain_application() {
                        app.attach_files(&paths);
                    }
                }
            ),
        );

        editor.connect_closure(
            "image-pasted",
            false,
            glib::closure_local!(
                #[weak(rename_to = window)]
                self,
                move |_: Editor, path: String| {
                    if let Some(app) = window.brain_application() {
                        app.attach_pasted_image(&path);
                    }
                }
            ),
        );

        editor.connect_closure(
            "tag-activated",
            false,
            glib::closure_local!(
                #[weak(rename_to = window)]
                self,
                move |_: Editor, tag: String| {
                    if let Some(app) = window.brain_application() {
                        app.filter_by_tag(Some(&tag));
                    }
                }
            ),
        );

        // Answered synchronously: the editor reads the candidates back as soon
        // as this returns, so the popover updates in the same turn.
        editor.connect_closure(
            "link-query",
            false,
            glib::closure_local!(
                #[weak(rename_to = window)]
                self,
                move |editor: Editor, query: String| {
                    let candidates = window
                        .brain_application()
                        .map(|app| app.link_candidates(&query))
                        .unwrap_or_default();
                    editor.set_link_candidates(&candidates);
                }
            ),
        );

        editor.connect_closure(
            "body-changed",
            false,
            glib::closure_local!(
                #[weak(rename_to = window)]
                self,
                move |_: Editor| {
                    window.update_status();
                    if let Some(app) = window.brain_application() {
                        app.note_edited();
                    }
                }
            ),
        );

        self.imp().split.replace(Some(split));
        self.imp().sidebar.replace(Some(sidebar));
        self.imp().editor.replace(Some(editor));
        self.imp().stack.replace(Some(stack));
        self.imp().toasts.replace(Some(toasts));
        self.imp().title.replace(Some(title));
        self.imp().status.replace(Some(status));
        self.imp().backlinks.replace(Some(backlinks));
        self.imp().backlinks_title.replace(Some(backlinks_title));
        self.imp().details.replace(Some(details));
        self.imp().detail_stack.replace(Some(detail_stack));
        self.imp().tags.replace(Some(tags));
        self.imp().sidebar_stack.replace(Some(sidebar_stack));
        self.imp().sidebar_title.replace(Some(sidebar_title));
        self.imp().clear_filter.replace(Some(clear_filter));
        self.imp().detail.replace(Some(detail));
        self.imp().banner.replace(Some(banner));
    }

    fn install_actions(&self) {
        let actions = gtk::gio::SimpleActionGroup::new();

        let new_note = gtk::gio::SimpleAction::new("new-note", None);
        new_note.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.prompt_new_note()
        ));
        actions.add_action(&new_note);

        let rename = gtk::gio::SimpleAction::new("rename-note", None);
        rename.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.prompt_rename()
        ));
        actions.add_action(&rename);

        let delete = gtk::gio::SimpleAction::new("delete-note", None);
        delete.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.prompt_delete()
        ));
        actions.add_action(&delete);

        let choose = gtk::gio::SimpleAction::new("choose-vault", None);
        choose.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.choose_vault()
        ));
        actions.add_action(&choose);

        let reload = gtk::gio::SimpleAction::new("reload", None);
        reload.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                if let Some(app) = window.brain_application() {
                    app.flush_open_note();
                    app.reload_vault();
                }
            }
        ));
        actions.add_action(&reload);

        let toggle_sidebar = gtk::gio::SimpleAction::new("toggle-sidebar", None);
        toggle_sidebar.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                if let Some(split) = window.imp().split.borrow().as_ref() {
                    split.set_show_sidebar(!split.shows_sidebar());
                }
            }
        ));
        actions.add_action(&toggle_sidebar);

        let toggle_reading = gtk::gio::SimpleAction::new("toggle-reading", None);
        toggle_reading.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.set_reading(!window.is_reading())
        ));
        actions.add_action(&toggle_reading);

        let toggle_backlinks = gtk::gio::SimpleAction::new("toggle-backlinks", None);
        toggle_backlinks.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                if let Some(detail) = window.imp().detail.borrow().as_ref() {
                    detail.set_show_sidebar(!detail.shows_sidebar());
                }
            }
        ));
        actions.add_action(&toggle_backlinks);

        let clear_filter = gtk::gio::SimpleAction::new("clear-filter", None);
        clear_filter.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                if let Some(app) = window.brain_application() {
                    app.filter_by_tag(None);
                }
            }
        ));
        actions.add_action(&clear_filter);

        let unused = gtk::gio::SimpleAction::new("unused-attachments", None);
        unused.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                if let Some(app) = window.brain_application() {
                    app.show_unused_attachments();
                }
            }
        ));
        actions.add_action(&unused);

        let shortcuts = gtk::gio::SimpleAction::new("shortcuts", None);
        shortcuts.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.show_shortcuts()
        ));
        actions.add_action(&shortcuts);

        for (name, mode) in [("quick-open", Mode::Title), ("search-text", Mode::Text)] {
            let action = gtk::gio::SimpleAction::new(name, None);
            action.connect_activate(clone!(
                #[weak(rename_to = window)]
                self,
                move |_, _| window.open_palette(mode)
            ));
            actions.add_action(&action);
        }

        // Ctrl+S is a no-op that flushes. Autosave is real, but people press it
        // anyway, and a shortcut that appears to do nothing is unnerving.
        let save = gtk::gio::SimpleAction::new("save", None);
        save.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                if let Some(app) = window.brain_application() {
                    app.flush_open_note();
                    app.save_now();
                }
                window.toast("Saved");
            }
        ));
        actions.add_action(&save);

        self.insert_action_group("win", Some(&actions));
        self.imp().actions.replace(Some(actions));
    }

    // ---- what the application tells the window to show ----

    pub fn set_notes(&self, notes: &[(NoteId, String)]) {
        if let Some(sidebar) = self.imp().sidebar.borrow().as_ref() {
            sidebar.set_notes(notes);
        }
    }

    /// Whether a `win.` action of this name is registered.
    ///
    /// An action named in a menu but never added to the group leaves a menu
    /// item that does nothing, and nothing warns about it.
    pub fn has_action(&self, name: &str) -> bool {
        use gtk::gio::prelude::ActionGroupExt;
        self.imp()
            .actions
            .borrow()
            .as_ref()
            .is_some_and(|actions| actions.has_action(name))
    }

    /// Whether the note is being read rather than edited.
    pub fn is_reading(&self) -> bool {
        self.imp()
            .reading_toggle
            .borrow()
            .as_ref()
            .is_some_and(gtk::ToggleButton::is_active)
    }

    /// Switch between reading and editing. Goes through the toggle so the
    /// header always says which mode you are in, however the mode was changed.
    pub fn set_reading(&self, reading: bool) {
        if let Some(toggle) = self.imp().reading_toggle.borrow().as_ref() {
            toggle.set_active(reading);
        }
    }

    /// Put the whole window into a mode: the editor, the formatting buttons,
    /// and what the next launch will open in.
    fn apply_reading(&self, reading: bool) {
        let imp = self.imp();
        if let Some(editor) = imp.editor.borrow().as_ref() {
            editor.set_reading(reading);
            if !reading {
                editor.grab_focus_to_text();
            }
        }
        if let Some(details) = imp.details.borrow().as_ref() {
            details.set_formatting_enabled(!reading);
        }
        if let Some(app) = self.brain_application() {
            app.remember_reading(reading);
        }
    }

    /// Show or hide the backlinks pane, for previews and tests.
    pub fn set_backlinks_shown(&self, shown: bool) {
        if let Some(detail) = self.imp().detail.borrow().as_ref() {
            detail.set_show_sidebar(shown);
        }
    }

    /// Collapse the split views, as the breakpoint does on a narrow screen.
    ///
    /// Exists so the narrow layout can be looked at offscreen, where there is
    /// no window for the breakpoint to measure.
    pub fn set_collapsed_for_test(&self, collapsed: bool) {
        let imp = self.imp();
        if let Some(split) = imp.split.borrow().as_ref() {
            split.set_collapsed(collapsed);
            split.set_show_sidebar(!collapsed);
        }
        if let Some(detail) = imp.detail.borrow().as_ref() {
            detail.set_collapsed(collapsed);
        }
    }

    /// The shortcuts dialog, built without presenting it, for previews.
    pub fn shortcuts_dialog_for_test() -> adw::ShortcutsDialog {
        Self::build_shortcuts()
    }

    /// The keyboard shortcuts, which are otherwise undiscoverable.
    ///
    /// Written out by hand rather than derived from the accel map: the map
    /// knows the keys but not which ones are worth telling someone about, nor
    /// what to call them.
    fn show_shortcuts(&self) {
        Self::build_shortcuts().present(Some(self));
    }

    fn build_shortcuts() -> adw::ShortcutsDialog {
        let dialog = adw::ShortcutsDialog::new();

        let sections: &[(&str, &[(&str, &str)])] = &[
            (
                "Notes",
                &[
                    ("<Control>n", "New note"),
                    ("<Control>s", "Save now"),
                    ("<Control>r", "Reload from disk"),
                    ("<Shift>F10", "Menu for the note in the list"),
                ],
            ),
            (
                "Finding Things",
                &[
                    ("<Control>k", "Go to note"),
                    ("<Control><Shift>f", "Search all notes"),
                    ("<Control>Return", "Follow the link or tag at the cursor"),
                ],
            ),
            (
                "View",
                &[
                    ("<Control>e", "Switch between reading and editing"),
                    ("F9", "Show or hide the sidebar"),
                    ("F10", "Show or hide backlinks"),
                    ("<Control>q", "Quit"),
                ],
            ),
        ];

        for (title, items) in sections {
            let section = adw::ShortcutsSection::new(Some(title));
            for (accelerator, description) in *items {
                section.add(adw::ShortcutsItem::new(description, accelerator));
            }
            dialog.add(section);
        }
        dialog
    }

    /// Open the search palette, building it the first time.
    fn open_palette(&self, mode: Mode) {
        let existing = self.imp().palette.borrow().clone();
        let palette = match existing {
            Some(palette) => palette,
            None => {
                let palette = Palette::new();

                palette.connect_closure(
                    "query-changed",
                    false,
                    glib::closure_local!(
                        #[weak(rename_to = window)]
                        self,
                        move |palette: Palette, query: String| {
                            let hits = window
                                .brain_application()
                                .map(|app| app.search(&query, palette.mode()))
                                .unwrap_or_default();
                            palette.set_hits(&hits);
                        }
                    ),
                );

                palette.connect_closure(
                    "chosen",
                    false,
                    glib::closure_local!(
                        #[weak(rename_to = window)]
                        self,
                        move |_: Palette, id: String| {
                            if let Some(app) = window.brain_application() {
                                app.open_note(&NoteId::from_relative(id));
                            }
                        }
                    ),
                );

                self.imp().palette.replace(Some(palette.clone()));
                palette
            }
        };
        palette.open(self, mode);
    }

    /// Tell the editor where the vault is, so embeds can be found on disk.
    pub fn set_vault_root(&self, root: Option<std::path::PathBuf>) {
        if let Some(editor) = self.imp().editor.borrow().as_ref() {
            editor.set_vault_root(root);
        }
    }

    /// Insert an `![[embed]]` for each attachment just added.
    pub fn insert_embeds(&self, names: &[String]) {
        if let Some(editor) = self.imp().editor.borrow().as_ref() {
            for name in names {
                editor.insert_embed(name);
            }
        }
    }

    /// List attachments no note refers to. Never deletes anything: an orphan
    /// is a file the user put there, and Brain is not the authority on it.
    pub fn show_unused_attachments(&self, names: &[String]) {
        let dialog = adw::AlertDialog::builder()
            .heading("Unused Attachments")
            .close_response("close")
            .build();
        dialog.add_response("close", "Close");

        if names.is_empty() {
            dialog.set_body("Every file in the attachments folder is referred to by a note.");
        } else {
            dialog.set_body(&format!(
                "{} file{} in the attachments folder {} referred to by no note. \
                 Nothing has been deleted — remove them in Files if you want them gone.",
                names.len(),
                if names.len() == 1 { "" } else { "s" },
                if names.len() == 1 { "is" } else { "are" },
            ));

            let list = gtk::ListBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .build();
            list.add_css_class("boxed-list");
            for name in names {
                let label = gtk::Label::builder()
                    .label(name)
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::Middle)
                    .margin_top(8)
                    .margin_bottom(8)
                    .margin_start(12)
                    .margin_end(12)
                    .build();
                list.append(&label);
            }
            let scroller = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .propagate_natural_height(true)
                .max_content_height(280)
                .child(&list)
                .build();
            dialog.set_extra_child(Some(&scroller));
        }

        dialog.present(Some(self));
    }

    /// Show every tag in the vault.
    pub fn set_tags(&self, tags: &[(String, usize)]) {
        if let Some(tree) = self.imp().tags.borrow().as_ref() {
            tree.set_tags(tags);
        }
    }

    /// Reflect which tag, if any, is filtering the note list.
    pub fn set_active_tag(&self, tag: Option<&str>) {
        let imp = self.imp();
        if let Some(tree) = imp.tags.borrow().as_ref() {
            tree.select(tag);
        }
        if let Some(title) = imp.sidebar_title.borrow().as_ref() {
            match tag {
                Some(tag) => {
                    title.set_title("Notes");
                    title.set_subtitle(&format!("#{tag}"));
                }
                None => {
                    title.set_title("Notes");
                    title.set_subtitle("");
                }
            }
        }
        if let Some(button) = imp.clear_filter.borrow().as_ref() {
            button.set_visible(tag.is_some());
        }
        // Choosing a tag is a request to see the notes carrying it, so show
        // them rather than leaving the user on the tag list to find their own
        // way back.
        if tag.is_some() {
            if let Some(stack) = imp.sidebar_stack.borrow().as_ref() {
                stack.set_visible_child_name("notes");
            }
        }
    }

    /// The save banner's text, if it is showing. For tests.
    pub fn save_error_for_test(&self) -> Option<String> {
        let banner = self.imp().banner.borrow().clone()?;
        banner.is_revealed().then(|| banner.title().to_string())
    }

    /// Drive a formatting request the way the panel does, for tests.
    pub fn request_format_for_test(&self, format: crate::model::markdown::Format) {
        if let Some(panel) = self.imp().details.borrow().as_ref() {
            panel.emit_by_name::<()>("format-requested", &[&DetailsPanel::format_name(format)]);
        }
    }

    /// Keep the right pane's header saying what is actually showing.
    ///
    /// The count belongs to the backlinks view, so it must not sit under the
    /// word "Details" — a subtitle describing the other tab reads as a bug.
    fn update_detail_title(&self) {
        let imp = self.imp();
        let (Some(title), Some(stack)) = (
            imp.backlinks_title.borrow().clone(),
            imp.detail_stack.borrow().clone(),
        ) else {
            return;
        };

        let backlinks = stack.visible_child_name().as_deref() == Some("backlinks");
        title.set_title(if backlinks { "Backlinks" } else { "Details" });
        title.set_subtitle(&if backlinks {
            match imp.backlink_count.get() {
                0 => "Nothing links here".to_string(),
                1 => "1 note".to_string(),
                count => format!("{count} notes"),
            }
        } else {
            String::new()
        });
    }

    /// Show the open note's properties.
    pub fn set_details(
        &self,
        note: Option<&NoteId>,
        tags: &[String],
        words: usize,
        created: Option<String>,
        updated: Option<String>,
    ) {
        if let Some(panel) = self.imp().details.borrow().as_ref() {
            panel.set_note(note, tags, words, created, updated);
        }
    }

    /// Show the notes linking to the open one.
    pub fn set_backlinks(&self, backlinks: &[(NoteId, String)]) {
        let imp = self.imp();
        if let Some(panel) = imp.backlinks.borrow().as_ref() {
            panel.set_backlinks(backlinks);
        }
        imp.backlink_count.set(backlinks.len());
        self.update_detail_title();
    }

    /// Offer to write a note that a link points at but that does not exist.
    pub fn confirm_create_note<F>(&self, title: &str, create: F)
    where
        F: Fn(String) + 'static,
    {
        let dialog = adw::AlertDialog::builder()
            .heading(format!("Create “{title}”?"))
            .body("Nothing in the vault answers to that name yet.")
            .close_response("cancel")
            .default_response("create")
            .build();
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("create", "Create");
        dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);

        let title = title.to_string();
        dialog.connect_response(None, move |_, response| {
            if response == "create" {
                create(title.clone());
            }
        });
        dialog.present(Some(self));
    }

    /// Ask which of several notes a link meant.
    pub fn ask_which_note<F>(&self, target: &str, candidates: &[NoteId], open: F)
    where
        F: Fn(NoteId) + 'static,
    {
        let dialog = adw::AlertDialog::builder()
            .heading(format!("Which “{target}”?"))
            .body("More than one note answers to that name.")
            .close_response("cancel")
            .build();
        dialog.add_response("cancel", "Cancel");

        // The full path is the only thing telling these apart, so that is what
        // the buttons say.
        for id in candidates {
            dialog.add_response(id.as_str(), id.as_str());
        }

        let by_id: Vec<NoteId> = candidates.to_vec();
        dialog.connect_response(None, move |_, response| {
            if let Some(id) = by_id.iter().find(|id| id.as_str() == response) {
                open(id.clone());
            }
        });
        dialog.present(Some(self));
    }

    /// Re-highlight a note in the list, without opening it.
    ///
    /// Rebuilding the list clears the selection, so anything that refreshes it
    /// while a note is open has to put the highlight back.
    pub fn select_note(&self, id: Option<&NoteId>) {
        if let Some(sidebar) = self.imp().sidebar.borrow().as_ref() {
            sidebar.select(id);
        }
    }

    /// Show a note, or none.
    pub fn show_note(&self, note: Option<(&NoteId, &str)>) {
        let imp = self.imp();
        let (Some(editor), Some(stack)) = (imp.editor.borrow().clone(), imp.stack.borrow().clone())
        else {
            return;
        };

        match note {
            Some((id, body)) => {
                editor.load(body);
                editor.set_editable(true);
                stack.set_visible_child_name("editor");
                if let Some(title) = imp.title.borrow().as_ref() {
                    title.set_title(id.title());
                    title.set_subtitle(id.folder().unwrap_or(""));
                }
                if let Some(sidebar) = imp.sidebar.borrow().as_ref() {
                    sidebar.select(Some(id));
                }
                editor.grab_focus_to_text();
            }
            None => {
                editor.load("");
                editor.set_editable(false);
                stack.set_visible_child_name("placeholder");
                if let Some(title) = imp.title.borrow().as_ref() {
                    title.set_title("Brain");
                    title.set_subtitle("");
                }
                if let Some(sidebar) = imp.sidebar.borrow().as_ref() {
                    sidebar.select(None);
                }
            }
        }
        self.update_status();
        self.update_sensitivity(note.is_some());
    }

    fn update_sensitivity(&self, has_note: bool) {
        for name in ["rename-note", "delete-note", "save"] {
            if let Some(action) = self
                .lookup_action(name)
                .and_downcast::<gtk::gio::SimpleAction>()
            {
                action.set_enabled(has_note);
            }
        }
    }

    fn update_status(&self) {
        let imp = self.imp();
        let (Some(editor), Some(status)) =
            (imp.editor.borrow().clone(), imp.status.borrow().clone())
        else {
            return;
        };
        let words = editor.word_count();
        status.set_text(&match words {
            1 => "1 word".to_string(),
            other => format!("{other} words"),
        });
    }

    /// The editor itself, for tests that drive it directly.
    pub fn editor(&self) -> Option<Editor> {
        self.imp().editor.borrow().clone()
    }

    /// The scan the editor is styled against, for tests.
    pub fn editor_parsed(&self) -> Option<crate::model::markdown::Parsed> {
        self.imp().editor.borrow().as_ref().map(Editor::parsed)
    }

    /// The text currently in the editor, for the application to persist.
    pub fn editor_body(&self) -> Option<String> {
        self.imp().editor.borrow().as_ref().map(Editor::body)
    }

    pub fn toast(&self, message: &str) {
        if let Some(toasts) = self.imp().toasts.borrow().as_ref() {
            toasts.add_toast(adw::Toast::new(message));
        }
    }

    /// Show or clear the "not saving" banner. `None` means saving works.
    pub fn set_save_error(&self, message: Option<&str>) {
        let Some(banner) = self.imp().banner.borrow().clone() else {
            return;
        };
        match message {
            Some(message) => {
                banner.set_title(message);
                banner.set_revealed(true);
            }
            None => banner.set_revealed(false),
        }
    }

    // ---- dialogs ----

    fn prompt_new_note(&self) {
        self.prompt_for_title("New Note", "Create", "", move |window, title| {
            if let Some(app) = window.brain_application() {
                app.create_note(&title);
            }
        });
    }

    fn prompt_rename(&self) {
        let Some(app) = self.brain_application() else {
            return;
        };
        let Some(current) = app.open_note_id() else {
            return;
        };
        let existing = current.title().to_string();
        self.prompt_for_title("Rename Note", "Rename", &existing, move |window, title| {
            if let Some(app) = window.brain_application() {
                app.rename_note(&title);
            }
        });
    }

    /// One dialog for both "new" and "rename": a title, a confirm button, and
    /// nothing else. Names are filenames, so the entry validates as you type.
    fn prompt_for_title<F>(&self, heading: &str, verb: &str, initial: &str, apply: F)
    where
        F: Fn(&BrainWindow, String) + 'static,
    {
        let dialog = adw::AlertDialog::builder()
            .heading(heading)
            .close_response("cancel")
            .default_response("confirm")
            .build();
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("confirm", verb);
        dialog.set_response_appearance("confirm", adw::ResponseAppearance::Suggested);

        let entry = adw::EntryRow::builder()
            .title("Title")
            .text(initial)
            .build();
        let group = adw::PreferencesGroup::new();
        group.add(&entry);
        dialog.set_extra_child(Some(&group));

        let validate = clone!(
            #[weak]
            dialog,
            #[weak]
            entry,
            move || {
                let text = entry.text();
                let name = text.trim();
                // The title becomes a filename. Rejecting these here is kinder
                // than letting the vault refuse the write afterwards.
                let ok = !name.is_empty()
                    && !name.contains('/')
                    && !name.starts_with('.')
                    && !name.contains('\0');
                dialog.set_response_enabled("confirm", ok);
                if ok || name.is_empty() {
                    entry.remove_css_class("error");
                } else {
                    entry.add_css_class("error");
                }
            }
        );
        entry.connect_changed(clone!(
            #[strong]
            validate,
            move |_| validate()
        ));
        validate();

        // Enter in the entry confirms, which takes both halves: `close` emits
        // the *close* response — "cancel" — and emitting "response" does not
        // close, because libadwaita closes from the button handler rather than
        // from the signal. Doing one without the other either threw the title
        // away or left the dialog on screen. The cancel that `close` emits is
        // ignored by the handler below, which acts only on "confirm".
        entry.connect_entry_activated(clone!(
            #[weak]
            dialog,
            move |_| {
                if dialog.is_response_enabled("confirm") {
                    dialog.close();
                    dialog.emit_by_name::<()>("response", &[&"confirm"]);
                }
            }
        ));

        dialog.connect_response(
            None,
            clone!(
                #[weak(rename_to = window)]
                self,
                #[weak]
                entry,
                move |_, response| {
                    if response != "confirm" {
                        return;
                    }
                    let title = entry.text().trim().to_string();
                    if !title.is_empty() {
                        apply(&window, title);
                    }
                }
            ),
        );

        dialog.present(Some(self));
        entry.grab_focus();
    }

    fn prompt_delete(&self) {
        let Some(app) = self.brain_application() else {
            return;
        };
        let Some(id) = app.open_note_id() else {
            return;
        };

        let dialog = adw::AlertDialog::builder()
            .heading(format!("Delete “{}”?", id.title()))
            .body("The file is removed from the vault. Links to it in other notes are left as they are.")
            .close_response("cancel")
            .default_response("cancel")
            .build();
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("delete", "Delete");
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);

        dialog.connect_response(
            None,
            clone!(
                #[weak(rename_to = window)]
                self,
                move |_, response| {
                    if response != "delete" {
                        return;
                    }
                    if let Some(app) = window.brain_application() {
                        app.delete_open_note();
                    }
                }
            ),
        );
        dialog.present(Some(self));
    }

    /// Pick the vault folder.
    ///
    /// `GtkFileDialog` goes through the file portal under Flatpak, and the
    /// document store keeps the grant across restarts — which is why the
    /// manifest needs no `--filesystem` at all.
    pub fn choose_vault(&self) {
        let dialog = gtk::FileDialog::builder()
            .title("Choose Vault Folder")
            .accept_label("Use This Folder")
            .modal(true)
            .build();

        dialog.select_folder(
            Some(self),
            gtk::gio::Cancellable::NONE,
            clone!(
                #[weak(rename_to = window)]
                self,
                move |result| {
                    let Ok(folder) = result else {
                        return; // cancelled, which is not an error
                    };
                    let Some(path) = folder.path() else {
                        window.toast("That folder is not on this filesystem");
                        return;
                    };
                    if let Some(app) = window.brain_application() {
                        app.set_vault(&path);
                    }
                }
            ),
        );
    }
}
