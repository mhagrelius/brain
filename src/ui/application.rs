//! The application: the GTK side of the notebook.
//!
//! What Brain *does* to a vault lives in [`brain_core::notebook::Notebook`].
//! What is left here is what only a toolkit can do — actions and accelerators,
//! the save tick, the file monitors, the worker threads, and turning a
//! notebook's outcome into a toast or a redraw.
//!
//! The rule has not moved, only its address: the notebook is the only thing
//! that writes a file or mutates the index, and every widget still reports
//! intent rather than acting on it. What changed is that the rule is now
//! testable without a display.
//!
//! # Borrowing
//!
//! The notebook lives in a `RefCell` and the window can re-enter this object
//! from a callback, so every method takes the borrow, gets its answer, and
//! drops it *before* touching a widget. `let outcome = { … borrow_mut() … };`
//! is the shape; a `match` whose scrutinee is still borrowing would panic the
//! first time an arm called back in.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::time::Duration;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib::{self, clone};

use crate::model::note::NoteId;
use crate::model::notebook::{Alert, Failed, Hit, Mode, Moved, Notebook, Renamed, Saved};
use crate::model::semantic;
use crate::model::tree::{Row, Sort};
use crate::model::vault::VaultError;
use crate::ui::{BrainWindow, Dragged, Watcher};
use crate::APP_ID;

/// How often a dirty note is written out. Long enough to be free while typing,
/// short enough that a hard crash loses a couple of seconds at worst.
const TICK: Duration = Duration::from_secs(2);

/// How long the vectors are allowed to lag the vault.
///
/// A save, a rename and the watcher noticing the same write all land within a
/// second of each other, and each one rescans. Waiting past the flurry means
/// embedding a note once instead of three times, and nothing on screen is
/// waiting for the result.
const CATCH_UP_DELAY: Duration = Duration::from_secs(5);

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct BrainApplication {
        /// Everything Brain knows about the vault. The only writer.
        pub notebook: RefCell<Notebook>,
        pub tick: RefCell<Option<glib::SourceId>>,
        pub window: RefCell<Option<BrainWindow>>,
        /// Watches the vault for changes made outside Brain. Dropped and
        /// rebuilt whenever the vault changes, which also cancels its monitors.
        pub watcher: RefCell<Option<Watcher>>,
        /// A catch-up pass is on a worker thread. A second one must not start
        /// beside it — they would both write the store — so a change arriving
        /// mid-pass sets `restack` and the pass runs again when it lands.
        pub catching_up: Cell<bool>,
        pub restack: Cell<bool>,
        pub catchup_tick: RefCell<Option<glib::SourceId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BrainApplication {
        const NAME: &'static str = "BrainApplication";
        type Type = super::BrainApplication;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for BrainApplication {}

    impl ApplicationImpl for BrainApplication {
        fn startup(&self) {
            // Chain up first: the toolkit initialises in the parent handler and
            // anything touching GTK before it is undefined.
            self.parent_startup();

            let obj = self.obj();
            if let Some(display) = gtk::gdk::Display::default() {
                crate::ui::load_stylesheet(&display);
            }
            obj.install_actions();
            obj.load_config();
            obj.start_tick();
        }

        fn activate(&self) {
            self.parent_activate();
            let obj = self.obj();

            // Taken out of the RefCell before the match, not inside its
            // scrutinee: a `borrow()` there lives for the whole match and the
            // `replace()` in the None arm would panic on it.
            let existing = self.window.borrow().clone();
            let window = match existing {
                Some(window) => window,
                None => {
                    let window = BrainWindow::new(&obj);
                    self.window.replace(Some(window.clone()));
                    window
                }
            };
            // Restore the size before presenting, so the window never appears
            // at one size and jumps to another.
            let (size, maximized, reading, root, has_vault) = {
                let notebook = self.notebook.borrow();
                let config = notebook.config();
                (
                    config.window_width.zip(config.window_height),
                    config.window_maximized,
                    config.reading_mode,
                    notebook.vault_root(),
                    notebook.has_vault(),
                )
            };
            if let Some((width, height)) = size {
                window.set_default_size(width.max(360), height.max(360));
            }
            if maximized {
                window.maximize();
            }
            window.set_reading(reading);
            window.present();
            window.set_vault_root(root);

            obj.refresh_notes();
            obj.restore_last_note();

            // No vault yet: ask, rather than showing an empty list that looks
            // like an empty vault.
            if !has_vault {
                window.choose_vault();
            }
        }

        /// Entry point for the desktop file's actions and for a second launch
        /// of an already-running instance.
        fn command_line(&self, command_line: &gtk::gio::ApplicationCommandLine) -> glib::ExitCode {
            let options = command_line.options_dict();
            let obj = self.obj();

            // Activating first means the window exists before anything is
            // asked of it, whether this is the first launch or the fifth.
            obj.activate();

            if options.contains("new-note") {
                if let Some(window) = obj.window() {
                    WidgetExt::activate_action(&window, "win.new-note", None).ok();
                }
            }
            if options.contains("search") {
                if let Some(window) = obj.window() {
                    WidgetExt::activate_action(&window, "win.quick-open", None).ok();
                }
            }
            glib::ExitCode::SUCCESS
        }

        fn shutdown(&self) {
            let obj = self.obj();
            obj.flush_open_note();
            obj.save_now();
            obj.remember_window();
            obj.save_config();
            if let Some(tick) = self.tick.take() {
                tick.remove();
            }
            self.watcher.replace(None);
            self.parent_shutdown();
        }
    }

    impl GtkApplicationImpl for BrainApplication {}
    impl AdwApplicationImpl for BrainApplication {}
}

glib::wrapper! {
    pub struct BrainApplication(ObjectSubclass<imp::BrainApplication>)
        @extends adw::Application, gtk::Application, gtk::gio::Application,
        @implements gtk::gio::ActionGroup, gtk::gio::ActionMap;
}

impl Default for BrainApplication {
    fn default() -> Self {
        Self::new()
    }
}

impl BrainApplication {
    pub fn new() -> Self {
        Self::with_application_id(APP_ID)
    }

    /// Construct under a given id.
    ///
    /// Tests use this so they never register under the real id — doing so would
    /// make the test process a D-Bus remote driving the developer's live app.
    pub fn with_application_id(id: &str) -> Self {
        let app: Self = glib::Object::builder()
            .property("application-id", id)
            // The desktop file's actions launch the binary with a flag, and a
            // second launch has to reach the running instance rather than
            // starting a new one.
            .property("flags", gtk::gio::ApplicationFlags::HANDLES_COMMAND_LINE)
            .build();

        app.add_main_option(
            "new-note",
            glib::Char::from(0),
            glib::OptionFlags::NONE,
            glib::OptionArg::None,
            "Start a new note",
            None,
        );
        app.add_main_option(
            "search",
            glib::Char::from(0),
            glib::OptionFlags::NONE,
            glib::OptionArg::None,
            "Open the search palette",
            None,
        );
        app
    }

    pub fn window(&self) -> Option<BrainWindow> {
        self.imp().window.borrow().clone()
    }

    fn install_actions(&self) {
        let quit = gtk::gio::SimpleAction::new("quit", None);
        quit.connect_activate(clone!(
            #[weak(rename_to = app)]
            self,
            move |_, _| app.quit()
        ));
        self.add_action(&quit);

        let about = gtk::gio::SimpleAction::new("about", None);
        about.connect_activate(clone!(
            #[weak(rename_to = app)]
            self,
            move |_, _| app.show_about()
        ));
        self.add_action(&about);

        self.set_accels_for_action("app.quit", &["<Control>q"]);
        self.set_accels_for_action("win.new-note", &["<Control>n"]);
        self.set_accels_for_action("win.save", &["<Control>s"]);
        self.set_accels_for_action("win.toggle-sidebar", &["F9"]);
        self.set_accels_for_action("win.reload", &["<Control>r"]);
        self.set_accels_for_action("win.toggle-backlinks", &["F10"]);
        self.set_accels_for_action("win.toggle-reading", &["<Control>e"]);
        self.set_accels_for_action("win.find", &["<Control>f"]);
        self.set_accels_for_action("win.quick-open", &["<Control>k"]);
        self.set_accels_for_action("win.search-text", &["<Control><Shift>f"]);
    }

    fn show_about(&self) {
        let about = adw::AboutDialog::builder()
            .application_name("Brain")
            .application_icon(APP_ID)
            .developer_name("Matthew Hagrelius")
            .version(env!("CARGO_PKG_VERSION"))
            .license_type(gtk::License::Gpl30)
            .comments("A Markdown notebook. Your notes stay plain files in a folder you own.")
            .build();
        about.present(
            self.window()
                .as_ref()
                .map(|w| w.upcast_ref::<gtk::Widget>()),
        );
    }

    // ---- configuration and the vault ----

    fn load_config(&self) {
        let problems = {
            let mut notebook = self.imp().notebook.borrow_mut();
            notebook.load_config(crate::model::config::default_path());
            notebook.rescan()
        };
        self.report_problems(&problems);
        self.schedule_catch_up();
        self.start_watching();
    }

    fn save_config(&self) {
        if let Err(error) = self.imp().notebook.borrow_mut().save_config() {
            // Losing this costs one trip through the folder chooser, so it is
            // worth a warning and not worth a dialog.
            glib::g_warning!("brain", "could not save config: {error}");
        }
    }

    /// Run the external-change path as the watcher would, for tests.
    pub fn absorb_external_changes_for_test(&self) {
        self.absorb_external_changes();
    }

    /// Record the window's size, so the next launch opens the same shape.
    pub fn remember_window(&self) {
        let Some(window) = self.window() else {
            return;
        };
        let maximized = window.is_maximized();
        let (width, height) = (window.width(), window.height());

        let mut notebook = self.imp().notebook.borrow_mut();
        let config = notebook.config_mut();
        config.window_maximized = maximized;
        // A maximised window reports the maximised size, which is not the size
        // to go back to when it is unmaximised again.
        //
        // Zero is not a size. This runs from `close_request`, where the window
        // still has one, and again from `shutdown`, where it may already be
        // unrealised and reports 0×0 — and the second call would otherwise
        // overwrite the first, so the app reopened at its minimum size.
        if !maximized && width > 0 && height > 0 {
            config.window_width = Some(width);
            config.window_height = Some(height);
        }
    }

    /// Remember which mode the window is in, for the next launch.
    pub fn remember_reading(&self, reading: bool) {
        self.imp().notebook.borrow_mut().config_mut().reading_mode = reading;
    }

    /// The mode that would be written to the config.
    pub fn remembered_reading(&self) -> bool {
        self.imp().notebook.borrow().config().reading_mode
    }

    /// The window size that would be written to the config.
    pub fn remembered_size(&self) -> Option<(i32, i32)> {
        let notebook = self.imp().notebook.borrow();
        let config = notebook.config();
        config.window_width.zip(config.window_height)
    }

    /// Point at a different vault folder.
    pub fn set_vault(&self, root: &Path) {
        self.flush_open_note();
        self.save_now();

        let problems = self.imp().notebook.borrow_mut().set_vault(root);
        self.save_config();
        self.report_problems(&problems);
        self.schedule_catch_up();
        self.start_watching();
        self.refresh_notes();

        if let Some(window) = self.window() {
            window.set_vault_root(Some(root.to_path_buf()));
        }
        self.show_open_note();
    }

    pub fn reload_vault(&self) {
        let problems = self.imp().notebook.borrow_mut().reload_vault();
        self.report_problems(&problems);
        self.schedule_catch_up();
        self.refresh_notes();
        self.show_open_note();
    }

    /// One file with the wrong permissions must not stop the app opening, but
    /// it must not be silent either.
    fn report_problems(&self, problems: &[VaultError]) {
        let Some(window) = self.window() else {
            return;
        };
        match problems.len() {
            0 => {}
            1 => window.toast(&format!("Could not read {}", problems[0])),
            count => window.toast(&format!("Could not read {count} files")),
        }
    }

    /// Watch the vault for changes made by anything other than Brain.
    fn start_watching(&self) {
        let imp = self.imp();
        // Dropping the old watcher cancels its monitors, which matters when
        // the vault has just been pointed somewhere else.
        imp.watcher.replace(None);

        let Some(root) = imp.notebook.borrow().vault_root() else {
            return;
        };
        let watcher = Watcher::new(
            &root,
            clone!(
                #[weak(rename_to = app)]
                self,
                move || app.absorb_external_changes()
            ),
        );
        imp.watcher.replace(Some(watcher));
    }

    /// Something changed the vault from outside. Take it on.
    ///
    /// The open note is the delicate part, and the notebook decides what
    /// happened to it; this only says so. A reload is the one outcome that
    /// changes what is on screen, and it is also the silent one — nothing was
    /// unsaved, so there is nothing to ask about.
    fn absorb_external_changes(&self) {
        let reloaded = matches!(
            self.imp().notebook.borrow_mut().absorb_external_changes(),
            crate::model::notebook::External::Reloaded
        );
        self.refresh_notes();
        self.schedule_catch_up();
        if reloaded {
            self.show_open_note();
        }
        self.refresh_banner();
    }

    /// Put whatever the notebook is worried about on the banner.
    ///
    /// One condition at a time, highest priority first, and at most one button
    /// — keeping what you typed is doing nothing, so only the other choice
    /// needs a control.
    fn refresh_banner(&self) {
        let Some(window) = self.window() else {
            return;
        };
        match self.imp().notebook.borrow().alert() {
            None => window.set_banner(None, None),
            Some(Alert::NotSaving(error)) => {
                window.set_banner(Some(&format!("Not saving: {error}")), None)
            }
            Some(Alert::Diverged(id)) => window.set_banner(
                Some(&format!(
                    "“{}” changed on disk — saving will overwrite that",
                    id.title()
                )),
                Some("Reload"),
            ),
            Some(Alert::Vanished(id)) => window.set_banner(
                Some(&format!(
                    "“{}” was deleted outside Brain and is still open here",
                    id.title()
                )),
                Some("Restore"),
            ),
        }
    }

    /// Take the banner up on what it is offering.
    pub fn resolve_alert(&self) {
        if self.imp().notebook.borrow_mut().resolve_alert() {
            self.refresh_notes();
            self.show_open_note();
            self.reselect();
        }
        self.refresh_banner();
    }

    // ---- vectors ----

    /// Bring the vectors level with the vault, a moment from now.
    ///
    /// Debounced, and deliberately unhurried. A save, a rename and an external
    /// change can arrive within a second of each other, and each one triggers a
    /// rescan; embedding after every one of them would mean embedding a note
    /// three times to reach the state it was already in. Nothing in the app is
    /// waiting on the result — search works without it — so a few seconds of
    /// lag costs nothing and saves the work.
    fn schedule_catch_up(&self) {
        let imp = self.imp();
        if let Some(pending) = imp.catchup_tick.take() {
            pending.remove();
        }
        if !imp.notebook.borrow().has_vault() {
            return;
        }
        let source = glib::timeout_add_local_once(
            CATCH_UP_DELAY,
            clone!(
                #[weak(rename_to = app)]
                self,
                move || {
                    app.imp().catchup_tick.replace(None);
                    app.catch_up_now();
                }
            ),
        );
        imp.catchup_tick.replace(Some(source));
    }

    /// Run one catch-up pass on a worker thread.
    ///
    /// The thread is handed copies of the index and the store and gives back a
    /// new store. Nothing is shared, so there is no lock and no way for the
    /// main loop to see a half-updated set of vectors: either the pass lands
    /// whole or the old store stands.
    fn catch_up_now(&self) {
        let imp = self.imp();
        if imp.catching_up.get() {
            // A pass is already out. Whatever changed will be caught by the one
            // queued behind it rather than by a second thread writing the same
            // store.
            imp.restack.set(true);
            return;
        }
        let (url, shared, index, store) = {
            let notebook = imp.notebook.borrow();
            let Some(url) = notebook.embedding_url(crate::ui::DEFAULT_EMBEDDING_URL) else {
                return; // semantic search is turned off
            };
            let (index, store) = notebook.catch_up_input();
            (url, notebook.shared_vectors(), index, store)
        };
        imp.catching_up.set(true);

        glib::spawn_future_local(clone!(
            #[weak(rename_to = app)]
            self,
            async move {
                let outcome = gtk::gio::spawn_blocking(move || {
                    use crate::model::semantic::Embedder;
                    let mut store = store;
                    let embedder = crate::ui::Llama::connect(&url)?;
                    // Both sessions are made on this thread and dropped with
                    // it, which is what libsoup asks for.
                    let shared = shared.map(|(url, token)| {
                        crate::ui::SharedVectors::new(&url, &token, &embedder.model())
                    });
                    let report = semantic::catch_up_sharing(
                        &mut store,
                        &index,
                        &embedder,
                        shared
                            .as_ref()
                            .map(|shared| shared as &dyn semantic::Shared),
                    );
                    Ok::<_, semantic::EmbedError>((store, report))
                })
                .await;

                app.imp().catching_up.set(false);
                // The vectors are only replaced wholesale, on the main thread,
                // which is the same discipline the index follows. Anything else
                // — no server, or one that refused — is not an error the user
                // needs a dialog about: search carries on lexically and the
                // next pass tries again.
                if let Ok(Ok((store, report))) = outcome {
                    app.imp().notebook.borrow_mut().absorb_vectors(store);
                    // A pass that embedded something may have changed what a
                    // search would return, and the palette is showing the
                    // previous answer.
                    if !report.is_quiet() {
                        if let Some(window) = app.window() {
                            window.refresh_palette();
                        }
                    }
                }
                if app.imp().restack.replace(false) {
                    app.schedule_catch_up();
                }
            }
        ));
    }

    /// Embed a query so the semantic half of the next search can use it.
    ///
    /// The lexical answer is already on screen by the time this is called; when
    /// the vector arrives the palette asks again and the fused answer replaces
    /// it. Typing a longer query in the meantime is fine — the result is stored
    /// against the query it came from, and a stale one simply never matches.
    fn embed_query(&self, query: &str) {
        let url = {
            let notebook = self.imp().notebook.borrow();
            if !notebook.has_vectors() {
                return; // nothing to compare a query against yet
            }
            match notebook.embedding_url(crate::ui::DEFAULT_EMBEDDING_URL) {
                Some(url) => url,
                None => return,
            }
        };
        let query = query.to_string();
        glib::spawn_future_local(clone!(
            #[weak(rename_to = app)]
            self,
            async move {
                let wanted = query.clone();
                let outcome = gtk::gio::spawn_blocking(move || {
                    use crate::model::semantic::Embedder;
                    // The query path, not the passage one: the model is trained
                    // to treat a question differently from the text answering
                    // it, and using the wrong side costs recall silently.
                    crate::ui::Llama::connect(&url)?.embed_query(&query)
                })
                .await;

                if let Ok(Ok(vector)) = outcome {
                    app.imp()
                        .notebook
                        .borrow_mut()
                        .set_query_vector(wanted, vector);
                    if let Some(window) = app.window() {
                        window.refresh_palette();
                    }
                }
            }
        ));
    }

    /// The vectors as they stand, for tests and for callers that want to search
    /// the vault themselves.
    pub fn vectors(&self) -> semantic::Store {
        self.imp().notebook.borrow().vectors()
    }

    // ---- what the sidebar shows ----

    pub fn listed_notes(&self) -> Vec<(NoteId, String)> {
        self.imp().notebook.borrow().listed_notes()
    }

    pub fn is_searching(&self) -> bool {
        self.imp().notebook.borrow().is_searching()
    }

    pub fn search_results(&self) -> Vec<(NoteId, String)> {
        self.imp().notebook.borrow().search_results()
    }

    pub fn sidebar_rows(&self) -> Vec<Row> {
        self.imp().notebook.borrow().sidebar_rows()
    }

    /// Open or close a folder, and make it where the next new note goes.
    pub fn toggle_folder(&self, path: &str) {
        self.imp().notebook.borrow_mut().toggle_folder(path);
        self.refresh_notes();
    }

    pub fn sort(&self) -> Sort {
        self.imp().notebook.borrow().sort()
    }

    pub fn set_sort(&self, sort: Sort) {
        self.imp().notebook.borrow_mut().set_sort(sort);
        self.refresh_notes();
        self.reselect();
    }

    /// Filter the sidebar by what was typed into its search entry.
    pub fn set_query(&self, query: &str) {
        if !self.imp().notebook.borrow_mut().set_query(query) {
            return;
        }
        self.refresh_notes();
        self.reselect();
    }

    pub fn tags(&self) -> Vec<(String, usize)> {
        self.imp().notebook.borrow().tags()
    }

    pub fn backlinks_of_open_note(&self) -> Vec<(NoteId, String)> {
        self.imp().notebook.borrow().backlinks_of_open_note()
    }

    fn refresh_notes(&self) {
        let Some(window) = self.window() else {
            return;
        };
        let (searching, results, rows, tags, filter) = {
            let notebook = self.imp().notebook.borrow();
            let searching = notebook.is_searching();
            (
                searching,
                if searching {
                    notebook.search_results()
                } else {
                    Vec::new()
                },
                if searching {
                    Vec::new()
                } else {
                    notebook.sidebar_rows()
                },
                notebook.tags(),
                notebook.active_tag(),
            )
        };

        if searching {
            window.set_result_count(Some(results.len()));
            window.set_results(&results);
        } else {
            window.set_rows(&rows);
            window.set_result_count(None);
        }
        window.set_tags(&tags);
        window.set_active_tag(filter.as_deref());
    }

    /// Put the highlight back on the open note after rebuilding the list.
    fn reselect(&self) {
        let open = self.imp().notebook.borrow().open_note_id();
        if let Some(window) = self.window() {
            window.select_note(open.as_ref());
        }
    }

    /// Show only notes carrying `tag`, or all of them.
    pub fn filter_by_tag(&self, tag: Option<&str>) {
        let known = self.imp().notebook.borrow_mut().filter_by_tag(tag);
        self.refresh_notes();

        if let (Some(window), Some(tag)) = (self.window(), tag) {
            if !known {
                window.toast(&format!(
                    "Nothing is tagged #{}",
                    tag.trim_start_matches('#')
                ));
            }
        }
    }

    // ---- the open note ----

    pub fn open_note_id(&self) -> Option<NoteId> {
        self.imp().notebook.borrow().open_note_id()
    }

    pub fn open_note(&self, id: &NoteId) {
        if self.imp().notebook.borrow().open_note_id().as_ref() == Some(id) {
            return;
        }
        // Switching notes flushes the one being left, rather than waiting for
        // the tick and racing it.
        self.flush_open_note();
        self.save_now();
        self.load_note(id);
    }

    fn load_note(&self, id: &NoteId) {
        let outcome = self.imp().notebook.borrow_mut().load_note(id);
        if let Err(error) = outcome {
            if let Some(window) = self.window() {
                window.toast(&format!("Could not open {error}"));
            }
        }
        self.show_open_note();
    }

    fn show_open_note(&self) {
        let Some(window) = self.window() else {
            return;
        };
        let open = self.imp().notebook.borrow().open_note_text();
        self.refresh_backlinks();
        match &open {
            Some((id, text)) => window.show_note(Some((id, text))),
            None => window.show_note(None),
        }
    }

    fn restore_last_note(&self) {
        if self
            .imp()
            .notebook
            .borrow_mut()
            .restore_last_note()
            .is_some()
        {
            self.show_open_note();
        }
    }

    /// The editor's text changed.
    pub fn note_edited(&self) {
        self.imp().notebook.borrow_mut().mark_edited();
    }

    /// Copy the editor's text into the open note. Does not write to disk.
    pub fn flush_open_note(&self) {
        let Some(window) = self.window() else {
            return;
        };
        let Some(body) = window.editor_body() else {
            return;
        };
        self.imp().notebook.borrow_mut().flush(&body);
    }

    /// Write the open note if it has unsaved changes.
    pub fn save_now(&self) {
        let outcome = self.imp().notebook.borrow_mut().save_now();
        match outcome {
            Saved::Clean => {}
            Saved::Written => {
                // Rebuilding the list clears the highlight, so put it back.
                self.refresh_notes();
                self.refresh_backlinks();
                self.reselect();
                self.refresh_banner();
            }
            // Saving is broken. Say so; the note stays dirty so the next tick
            // tries again — a note that failed to save must not be quietly
            // forgotten.
            Saved::Failed(_) => self.refresh_banner(),
        }
    }

    fn start_tick(&self) {
        let source = glib::timeout_add_local(
            TICK,
            clone!(
                #[weak(rename_to = app)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || {
                    app.flush_open_note();
                    app.save_now();
                    glib::ControlFlow::Continue
                }
            ),
        );
        self.imp().tick.replace(Some(source));
    }

    // ---- links ----

    /// Candidates for a `[[` completion, by title, alias and path.
    pub fn link_candidates(&self, query: &str) -> Vec<String> {
        self.imp().notebook.borrow().link_candidates(query)
    }

    /// Follow a `[[link]]`.
    ///
    /// A target with nothing behind it is offered as a note to write, because
    /// writing the link before the note is the normal order of things.
    pub fn follow_link(&self, target: &str) {
        use crate::model::index::Resolution;
        let resolution = self.imp().notebook.borrow().resolve_link(target);
        match resolution {
            Resolution::Note(id) => self.open_note(&id),
            Resolution::Missing => self.offer_to_create(target),
            Resolution::Ambiguous(candidates) => self.ask_which(target, &candidates),
        }
    }

    fn offer_to_create(&self, target: &str) {
        let Some(window) = self.window() else {
            return;
        };
        window.confirm_create_note(
            target,
            clone!(
                #[weak(rename_to = app)]
                self,
                move |title: String| app.create_note(&title)
            ),
        );
    }

    /// Two notes answer to this name, so ask rather than guess — a link that
    /// silently points at the wrong note never tells you it did.
    fn ask_which(&self, target: &str, candidates: &[NoteId]) {
        let Some(window) = self.window() else {
            return;
        };
        window.ask_which_note(
            target,
            candidates,
            clone!(
                #[weak(rename_to = app)]
                self,
                move |id: NoteId| app.open_note(&id)
            ),
        );
    }

    fn refresh_backlinks(&self) {
        let Some(window) = self.window() else {
            return;
        };
        let (backlinks, open, details) = {
            let notebook = self.imp().notebook.borrow();
            let details = notebook.open_note().map(|note| {
                (
                    note.tags(),
                    note.body.split_whitespace().count(),
                    note.frontmatter
                        .as_ref()
                        .and_then(|f| f.created)
                        .map(|date| date.to_string()),
                    note.frontmatter
                        .as_ref()
                        .and_then(|f| f.updated)
                        .map(|date| date.to_string()),
                )
            });
            (
                notebook.backlinks_of_open_note(),
                notebook.open_note_id(),
                details,
            )
        };
        window.set_backlinks(&backlinks);

        let (tags, words, created, updated) = details.unwrap_or_default();
        window.set_details(open.as_ref(), &tags, words, created, updated);
    }

    /// Answer a palette query.
    pub fn search(&self, query: &str, mode: Mode) -> Vec<Hit> {
        let (hits, wants_embedding) = self.imp().notebook.borrow().search(query, mode);
        if wants_embedding && mode == Mode::Text {
            self.embed_query(query);
        }
        hits
    }

    // ---- attachments ----

    /// Copy dropped files into the vault and embed them.
    pub fn attach_files(&self, paths: &[String]) {
        let Some(window) = self.window() else {
            return;
        };
        let attached = self.imp().notebook.borrow_mut().attach_files(paths);

        if !attached.names.is_empty() {
            window.insert_embeds(&attached.names);
            // The embed only points at a file; the save tick writes the note.
            self.flush_open_note();
        }
        if attached.failures > 0 {
            let failures = attached.failures;
            window.toast(&format!(
                "Could not copy {failures} file{}",
                if failures == 1 { "" } else { "s" }
            ));
        }
    }

    /// Take a pasted image out of the temporary file the editor wrote.
    pub fn attach_pasted_image(&self, path: &str) {
        let path = PathBuf::from(path);
        // Named for when it was pasted, since a clipboard image has no name of
        // its own and "image.png" would collide with every other paste.
        let stamped = path.with_file_name(format!(
            "Pasted {}.png",
            chrono::Local::now().format("%Y-%m-%d %H%M%S")
        ));
        let source = match std::fs::rename(&path, &stamped) {
            Ok(()) => stamped,
            Err(_) => path.clone(),
        };

        self.attach_files(&[source.to_string_lossy().to_string()]);
        // The temporary file has been copied into the vault; leaving it in
        // /tmp is litter.
        let _ = std::fs::remove_file(&source);
    }

    /// Files in `attachments/` that no note refers to.
    pub fn show_unused_attachments(&self) {
        let Some(window) = self.window() else {
            return;
        };
        let unused = self.imp().notebook.borrow().unused_attachments();
        window.show_unused_attachments(&unused);
    }

    // ---- creating, renaming, deleting ----

    pub fn create_note(&self, title: &str) {
        let folder = self.imp().notebook.borrow().current_folder();
        self.create_note_in(&folder, title);
    }

    /// Write a new note in a named folder.
    pub fn create_note_in(&self, folder: &str, title: &str) {
        self.flush_open_note();
        self.save_now();

        let outcome = self
            .imp()
            .notebook
            .borrow_mut()
            .create_note_in(folder, title);
        match outcome {
            Ok(_) => {
                self.refresh_notes();
                self.show_open_note();
            }
            Err(Failed::NoVault) => {}
            Err(error) => self.complain(&format!("Could not create the note: {error}")),
        }
    }

    /// Rename the open note and repoint every link that pointed at it.
    pub fn rename_note(&self, title: &str) {
        self.flush_open_note();
        self.save_now();

        let outcome = self.imp().notebook.borrow_mut().rename_note(title);
        match outcome {
            Renamed::Unchanged => {}
            Renamed::Failed(error) => self.complain(&format!("Could not rename: {error}")),
            Renamed::Done { links, .. } => {
                self.refresh_notes();
                self.show_open_note();
                if let Some(window) = self.window() {
                    match links {
                        0 => window.toast("Renamed"),
                        1 => window.toast("Renamed, and updated 1 link"),
                        count => window.toast(&format!("Renamed, and updated {count} links")),
                    }
                }
            }
        }
    }

    pub fn delete_open_note(&self) {
        let outcome = self.imp().notebook.borrow_mut().delete_open_note();
        match outcome {
            Err(Failed::NoVault) => {}
            Err(error) => self.complain(&format!("Could not delete: {error}")),
            Ok(id) => {
                self.refresh_notes();
                self.show_open_note();
                if let Some(window) = self.window() {
                    window.toast(&format!("Deleted “{}”", id.title()));
                }
            }
        }
    }

    // ---- folders ----

    /// The folders in the vault, for the dialogs that ask which one.
    pub fn folders(&self) -> Vec<String> {
        self.imp().notebook.borrow().folders()
    }

    /// Make a folder where a new note would go.
    pub fn create_folder_here(&self, name: &str) {
        let parent = self.imp().notebook.borrow().current_folder();
        self.create_folder(&parent, name);
    }

    pub fn create_folder(&self, parent: &str, name: &str) {
        let outcome = self.imp().notebook.borrow_mut().create_folder(parent, name);
        match outcome {
            Ok(_) => self.refresh_notes(),
            Err(Failed::NoVault) => {}
            Err(error) => self.complain(&format!("Could not create the folder: {error}")),
        }
    }

    /// Rename a folder in place, keeping everything under it.
    pub fn rename_folder(&self, path: &str, name: &str) {
        self.flush_open_note();
        self.save_now();
        let outcome = self.imp().notebook.borrow_mut().rename_folder(path, name);
        let parent = path
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .unwrap_or("");
        let to = if parent.is_empty() {
            name.to_string()
        } else {
            format!("{parent}/{name}")
        };
        self.report_relocation(outcome, &to, "Renamed");
    }

    pub fn delete_folder(&self, path: &str) {
        let outcome = self.imp().notebook.borrow_mut().delete_folder(path);
        match outcome {
            Ok(()) => {
                self.refresh_notes();
                if let Some(window) = self.window() {
                    window.toast(&format!("Deleted “{path}”"));
                }
            }
            Err(Failed::NoVault) => {}
            Err(error) => self.complain(&format!("Could not delete the folder: {error}")),
        }
    }

    /// Something was dragged onto a folder. Move it, or say why not.
    pub fn move_dropped(&self, payload: &str, destination: &str) {
        match crate::ui::sidebar::dragged(payload) {
            Some(Dragged::Note(id)) => self.move_note(&id, destination),
            Some(Dragged::Folder(path)) => {
                let name = path.rsplit('/').next().unwrap_or(&path).to_string();
                let to = if destination.is_empty() {
                    name
                } else {
                    format!("{destination}/{name}")
                };
                self.flush_open_note();
                self.save_now();
                let outcome = self.imp().notebook.borrow_mut().relocate_folder(&path, &to);
                self.report_relocation(outcome, &to, "Moved");
            }
            // Something dragged in from another app. Notes are files, so this
            // could one day mean "import", but silently doing nothing is
            // better than guessing at it.
            None => {}
        }
    }

    /// The tail of a folder move or rename: the notebook has already rebuilt
    /// its index, so this is redraw and wording only.
    fn report_relocation(&self, outcome: Result<(), Failed>, to: &str, verb: &str) {
        match outcome {
            Ok(()) => {
                self.schedule_catch_up();
                self.refresh_notes();
                self.show_open_note();
                self.reselect();
                if let Some(window) = self.window() {
                    window.toast(&format!("{verb} “{to}”"));
                }
            }
            Err(Failed::NoVault) => {}
            Err(error) => self.complain(&format!("Could not move the folder: {error}")),
        }
    }

    /// Move one note into a folder, keeping its title.
    pub fn move_note(&self, id: &NoteId, destination: &str) {
        // The note may be the open one with unsaved edits, and the file is
        // about to move out from under the save tick.
        if self.imp().notebook.borrow().open_note_id().as_ref() == Some(id) {
            self.flush_open_note();
            self.save_now();
        }

        let outcome = self.imp().notebook.borrow_mut().move_note(id, destination);
        match outcome {
            Moved::Unchanged => {}
            Moved::Failed(error) => self.complain(&format!("Could not move: {error}")),
            Moved::Done { to, destination } => {
                self.refresh_notes();
                self.show_open_note();
                if let Some(window) = self.window() {
                    window.toast(&match destination.as_str() {
                        "" => format!("Moved “{}” to the vault root", to.title()),
                        folder => format!("Moved “{}” to {folder}", to.title()),
                    });
                }
            }
        }
    }

    fn complain(&self, message: &str) {
        if let Some(window) = self.window() {
            window.toast(message);
        }
    }
}
