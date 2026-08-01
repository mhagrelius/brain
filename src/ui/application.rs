//! The application: owns the vault, the index, and the save tick.
//!
//! Everything that writes a file funnels through here. The window reports what
//! the user did; this object applies it to the vault, updates the index, and
//! pushes the result back down. Nothing else calls `write`, so there is exactly
//! one place a note can be lost.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::time::Duration;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib::{self, clone};

use crate::model::bm25::Bm25;
use crate::model::config::Config;
use crate::model::index::{Index, Resolution};
use crate::model::markdown;
use crate::model::note::{Note, NoteId};
use crate::model::search;
use crate::model::semantic;
use crate::model::tree::{self, Listed, Row, Sort};
use crate::model::vault::{Vault, VaultError};
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
        pub config: RefCell<Config>,
        pub config_path: RefCell<PathBuf>,
        pub vault: RefCell<Option<Vault>>,
        pub index: RefCell<Index>,
        /// The note in the editor, if any.
        pub open: RefCell<Option<NoteId>>,
        /// The open note as it stands, including unsaved edits.
        pub buffer: RefCell<Option<Note>>,
        /// The text of the open note as it last stood *on disk*, whether Brain
        /// put it there or read it from there. The watcher cannot tell whose
        /// write it is reporting, so this is what tells Brain's own saves apart
        /// from somebody else's edit.
        pub on_disk: RefCell<Option<String>>,
        pub dirty: Cell<bool>,
        pub tick: RefCell<Option<glib::SourceId>>,
        pub window: RefCell<Option<BrainWindow>>,
        /// The tag filtering the note list, if any.
        pub filter: RefCell<Option<String>>,
        /// What the sidebar's search entry holds. While it is not empty the
        /// sidebar shows results rather than the tree.
        pub query: RefCell<String>,
        /// Which folders are open in the sidebar.
        pub expanded: RefCell<std::collections::BTreeSet<String>>,
        pub sort: Cell<Sort>,
        /// The folder a new note goes in: whichever was last chosen in the
        /// sidebar, falling back to the open note's.
        pub target_folder: RefCell<Option<String>>,
        /// Watches the vault for changes made outside Brain. Dropped and
        /// rebuilt whenever the vault changes, which also cancels its monitors.
        pub watcher: RefCell<Option<Watcher>>,
        /// BM25 counts over the current index, rebuilt with it. Cheap enough to
        /// rebuild and wrong enough to keep if it is not.
        pub lexical: RefCell<Bm25>,
        /// The vectors, and where they are cached. Derived data: losing the
        /// file costs one pass of embedding.
        pub vectors: RefCell<semantic::Store>,
        pub vectors_path: RefCell<Option<PathBuf>>,
        /// A catch-up pass is on a worker thread. A second one must not start
        /// beside it — they would both write the store — so a change arriving
        /// mid-pass sets `restack` and the pass runs again when it lands.
        pub catching_up: Cell<bool>,
        pub restack: Cell<bool>,
        pub catchup_tick: RefCell<Option<glib::SourceId>>,
        /// The last query embedded, and its vector. One entry, because the only
        /// query worth having a vector for is the one in the palette now.
        pub query_vector: RefCell<Option<(String, Vec<f32>)>>,
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
            {
                let config = self.config.borrow();
                if let (Some(width), Some(height)) = (config.window_width, config.window_height) {
                    window.set_default_size(width.max(360), height.max(360));
                }
                if config.window_maximized {
                    window.maximize();
                }
            }
            // Outside the borrow above: this reaches back into the config to
            // record the mode, and would deadlock against it.
            let reading = self.config.borrow().reading_mode;
            window.set_reading(reading);
            window.present();

            window.set_vault_root(
                self.vault
                    .borrow()
                    .as_ref()
                    .map(|vault| vault.root().to_path_buf()),
            );
            obj.refresh_notes();
            obj.restore_last_note();

            // No vault yet: ask, rather than showing an empty list that looks
            // like an empty vault.
            if self.vault.borrow().is_none() {
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
        let imp = self.imp();
        let path = crate::model::config::default_path();
        let (config, _outcome) = Config::load(&path);
        imp.config_path.replace(path);

        if let Some(root) = config.vault.clone() {
            if root.is_dir() {
                imp.vault.replace(Some(Vault::new(root)));
            }
        }
        imp.sort.set(Sort::from_name(&config.sort));
        imp.expanded
            .replace(config.expanded_folders.iter().cloned().collect());
        imp.config.replace(config);
        self.load_vectors();
        self.rescan();
        self.start_watching();
    }

    fn save_config(&self) {
        let imp = self.imp();
        let path = imp.config_path.borrow().clone();
        {
            let mut config = imp.config.borrow_mut();
            config.sort = imp.sort.get().as_str().to_string();
            config.expanded_folders = imp.expanded.borrow().iter().cloned().collect();
        }
        let config = imp.config.borrow().clone();
        if let Err(error) = config.save(&path) {
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
        let imp = self.imp();
        let mut config = imp.config.borrow_mut();
        config.window_maximized = window.is_maximized();
        // A maximised window reports the maximised size, which is not the size
        // to go back to when it is unmaximised again.
        //
        // Zero is not a size. This runs from `close_request`, where the window
        // still has one, and again from `shutdown`, where it may already be
        // unrealised and reports 0×0 — and the second call would otherwise
        // overwrite the first, so the app reopened at its minimum size.
        if !window.is_maximized() {
            let (width, height) = (window.width(), window.height());
            if width > 0 && height > 0 {
                config.window_width = Some(width);
                config.window_height = Some(height);
            }
        }
    }

    /// Remember which mode the window is in, for the next launch.
    pub fn remember_reading(&self, reading: bool) {
        self.imp().config.borrow_mut().reading_mode = reading;
    }

    /// The mode that would be written to the config.
    pub fn remembered_reading(&self) -> bool {
        self.imp().config.borrow().reading_mode
    }

    /// The window size that would be written to the config.
    pub fn remembered_size(&self) -> Option<(i32, i32)> {
        let config = self.imp().config.borrow();
        config.window_width.zip(config.window_height)
    }

    /// Point at a different vault folder.
    pub fn set_vault(&self, root: &Path) {
        let imp = self.imp();
        self.flush_open_note();
        self.save_now();

        imp.vault.replace(Some(Vault::new(root)));
        imp.open.replace(None);
        imp.buffer.replace(None);
        imp.config.borrow_mut().vault = Some(root.to_path_buf());
        imp.config.borrow_mut().last_note = None;
        self.save_config();

        // Before the rescan: the rescan schedules a catch-up, and it should
        // start from whatever this vault already had embedded rather than from
        // the previous vault's vectors.
        self.load_vectors();
        self.rescan();
        self.start_watching();
        self.refresh_notes();
        if let Some(window) = self.window() {
            window.set_vault_root(Some(root.to_path_buf()));
        }
        self.show_open_note();
    }

    pub fn reload_vault(&self) {
        self.rescan();
        self.refresh_notes();

        // Taken out of the RefCell before anything else runs: `load_note`
        // replaces `open`, and holding the borrow across it panics.
        let open = self.imp().open.borrow().clone();
        // The open note may have been renamed or deleted outside the app.
        match open.filter(|id| self.imp().index.borrow().contains(id)) {
            Some(id) => self.load_note(&id),
            None => {
                self.imp().open.replace(None);
                self.imp().buffer.replace(None);
                self.show_open_note();
            }
        }
    }

    /// Watch the vault for changes made by anything other than Brain.
    fn start_watching(&self) {
        let imp = self.imp();
        // Dropping the old watcher cancels its monitors, which matters when
        // the vault has just been pointed somewhere else.
        imp.watcher.replace(None);

        let Some(vault) = imp.vault.borrow().clone() else {
            return;
        };
        let watcher = Watcher::new(
            vault.root(),
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
    /// The open note is the delicate part: reloading it would throw away
    /// whatever is being typed. So an edited note keeps what is in the editor
    /// and says the file moved underneath it; an unedited one is reloaded
    /// silently, which is what makes `git checkout` feel right.
    fn absorb_external_changes(&self) {
        let imp = self.imp();
        let open = imp.open.borrow().clone();

        self.rescan();
        self.refresh_notes();

        let Some(id) = open else {
            return;
        };
        let Some(window) = self.window() else {
            return;
        };

        if !imp.index.borrow().contains(&id) {
            window.set_save_error(Some(&format!(
                "“{}” was deleted or moved outside Brain. It is still open here.",
                id.title()
            )));
            return;
        }

        let Some(note) = imp
            .vault
            .borrow()
            .as_ref()
            .and_then(|vault| vault.read(&id).ok())
        else {
            return;
        };
        let text = note.to_text();

        // An event is not a change. The watcher fires for Brain's own saves as
        // loudly as for anyone else's, so what counts is whether the file
        // differs from what Brain last put there or read from there —
        // otherwise typing raises a "changed on disk" warning every two
        // seconds, about your own keystrokes.
        if imp.on_disk.borrow().as_deref() == Some(text.as_str()) {
            return;
        }
        imp.on_disk.replace(Some(text));

        if imp.dirty.get() {
            window.set_save_error(Some(&format!(
                "“{}” changed on disk. Saving will overwrite that.",
                id.title()
            )));
            return;
        }

        // Nothing unsaved, so the file is the truth.
        imp.buffer.replace(Some(note));
        window.set_save_error(None);
        self.show_open_note();
    }

    fn rescan(&self) {
        let imp = self.imp();
        let Some(vault) = imp.vault.borrow().clone() else {
            imp.index.replace(Index::default());
            imp.lexical.replace(Bm25::default());
            return;
        };

        let (notes, problems) = vault.scan();
        imp.index.replace(Index::build(&notes));
        imp.lexical.replace(Bm25::build(&imp.index.borrow()));
        // Whatever changed — a save here, a `git pull` behind the app's back,
        // or a folder dragged in Nautilus — the vectors are now behind the
        // vault by exactly the difference the planner will find.
        self.schedule_catch_up();

        // One file with the wrong permissions must not stop the app opening,
        // but it must not be silent either.
        if let Some(window) = self.window() {
            match problems.len() {
                0 => {}
                1 => window.toast(&format!("Could not read {}", problems[0])),
                count => window.toast(&format!("Could not read {count} files")),
            }
        }
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
        if imp.vault.borrow().is_none() {
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
        let Some(url) = self.embedding_url() else {
            return; // semantic search is turned off
        };
        let index = imp.index.borrow().clone();
        let store = imp.vectors.borrow().clone();
        imp.catching_up.set(true);

        glib::spawn_future_local(clone!(
            #[weak(rename_to = app)]
            self,
            async move {
                let outcome = gtk::gio::spawn_blocking(move || {
                    let mut store = store;
                    let embedder = crate::ui::Llama::connect(&url)?;
                    let report = semantic::catch_up(&mut store, &index, &embedder);
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
                    app.absorb_vectors(store, report);
                }
                if app.imp().restack.replace(false) {
                    app.schedule_catch_up();
                }
            }
        ));
    }

    /// Take on the store a pass produced, and write it out.
    fn absorb_vectors(&self, store: semantic::Store, report: semantic::Report) {
        let imp = self.imp();
        imp.vectors.replace(store);
        if let Some(path) = imp.vectors_path.borrow().clone() {
            // A cache that could not be written is not worth a word to the
            // user: the vectors are in memory and work, and the next pass
            // writes them again.
            let _ = imp.vectors.borrow().save(&path);
        }

        // A pass that embedded something may have changed what a search would
        // return, and the palette is showing the previous answer.
        if !report.is_quiet() {
            if let Some(window) = self.window() {
                window.refresh_palette();
            }
        }
    }

    /// Where the embedding server is, or `None` when semantic search is off.
    ///
    /// Off is a configured empty string, not a missing key: a vault with no
    /// server reachable behaves the same as one with the feature turned off,
    /// and the difference is only whether Brain keeps trying.
    fn embedding_url(&self) -> Option<String> {
        let configured = self.imp().config.borrow().embedding_url.clone();
        match configured {
            Some(url) if url.trim().is_empty() => None,
            Some(url) => Some(url),
            None => Some(crate::ui::DEFAULT_EMBEDDING_URL.to_string()),
        }
    }

    /// Point the vector store at a vault, reading whatever was cached for it.
    fn load_vectors(&self) {
        let imp = self.imp();
        let Some(vault) = imp.vault.borrow().clone() else {
            imp.vectors.replace(semantic::Store::default());
            imp.vectors_path.replace(None);
            return;
        };
        let path = semantic::default_store_path(vault.root());
        imp.vectors.replace(semantic::Store::load(&path));
        imp.vectors_path.replace(Some(path));
    }

    /// Embed a query so the semantic half of the next search can use it.
    ///
    /// The lexical answer is already on screen by the time this is called; when
    /// the vector arrives the palette asks again and the fused answer replaces
    /// it. Typing a longer query in the meantime is fine — the result is stored
    /// against the query it came from, and a stale one simply never matches.
    fn embed_query(&self, query: &str) {
        let Some(url) = self.embedding_url() else {
            return;
        };
        if self.imp().vectors.borrow().is_empty() {
            return; // nothing to compare a query against yet
        }
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
                    app.imp().query_vector.replace(Some((wanted, vector)));
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
        self.imp().vectors.borrow().clone()
    }

    /// The notes the sidebar is showing: every note, or the ones carrying the
    /// active tag, or the ones matching the search — in path order so folders
    /// group and nothing ever reshuffles, unless a search has ranked them.
    pub fn listed_notes(&self) -> Vec<(NoteId, String)> {
        if self.is_searching() {
            return self.search_results();
        }
        let index = self.imp().index.borrow();
        let mut notes: Vec<(NoteId, String)> = self
            .tagged_ids()
            .into_iter()
            .map(|id| {
                let excerpt = index.excerpt(&id).to_string();
                (id, excerpt)
            })
            .collect();
        notes.sort_by(|a, b| a.0.cmp(&b.0));
        notes
    }

    /// The ids the tag filter leaves, before any search.
    fn tagged_ids(&self) -> Vec<NoteId> {
        let index = self.imp().index.borrow();
        match self.imp().filter.borrow().as_ref() {
            Some(tag) => index.notes_tagged(tag),
            None => index.ids().cloned().collect(),
        }
    }

    /// Whether the sidebar is showing results rather than the tree.
    pub fn is_searching(&self) -> bool {
        !self.imp().query.borrow().trim().is_empty()
    }

    /// What the sidebar search matched, best first.
    ///
    /// Titles before text, because someone typing into a list of notes is
    /// usually reaching for one they can name. A note matched by both appears
    /// once, at its title rank, with the matching line as its excerpt so the
    /// row says *why* it is there.
    pub fn search_results(&self) -> Vec<(NoteId, String)> {
        let imp = self.imp();
        let query = imp.query.borrow().trim().to_string();
        if query.is_empty() {
            return Vec::new();
        }
        let index = imp.index.borrow();
        let allowed: std::collections::BTreeSet<NoteId> = self.tagged_ids().into_iter().collect();

        let mut out: Vec<(NoteId, String)> = Vec::new();
        for matched in search::by_title(&index, &query, 50) {
            if allowed.contains(&matched.id) {
                out.push((matched.id.clone(), index.excerpt(&matched.id).to_string()));
            }
        }
        for matched in search::by_text(&index, &query, 50) {
            if !allowed.contains(&matched.id) || out.iter().any(|(id, _)| id == &matched.id) {
                continue;
            }
            let excerpt = matched
                .snippets
                .first()
                .map(|snippet| snippet.text.clone())
                .unwrap_or_default();
            out.push((matched.id.clone(), excerpt));
        }
        out
    }

    /// The sidebar's tree, folders and all.
    pub fn sidebar_rows(&self) -> Vec<Row> {
        let imp = self.imp();
        let sort = imp.sort.get();
        let vault = imp.vault.borrow().clone();
        let index = imp.index.borrow();

        let notes: Vec<Listed> = self
            .tagged_ids()
            .into_iter()
            .map(|id| {
                // Timestamps are read from disk only when something is going to
                // sort by them. By name — the default — the sidebar refreshes
                // on every save, and stat-ing the vault each time to sort by a
                // field nobody asked for is work for nothing.
                let (modified, created) = match (sort, &vault) {
                    (Sort::Name, _) | (_, None) => (0, 0),
                    (_, Some(vault)) => vault.times(&id),
                };
                Listed {
                    excerpt: index.excerpt(&id).to_string(),
                    id,
                    modified,
                    created,
                }
            })
            .collect();

        // A tag filter is a question about notes, not about folders. So the
        // empty folders drop out of the tree while one is on rather than
        // sitting there claiming to hold something, and every folder that does
        // hold a match is opened — a filter whose results are behind a chevron
        // is a filter that looks like it found nothing.
        let filtered = imp.filter.borrow().is_some();
        let folders = match (filtered, &vault) {
            (false, Some(vault)) => vault.folders(),
            _ => Vec::new(),
        };
        let expanded = if filtered {
            notes
                .iter()
                .flat_map(|note| ancestors(note.id.folder().unwrap_or("")))
                .collect()
        } else {
            imp.expanded.borrow().clone()
        };
        tree::rows(&notes, &folders, &expanded, sort)
    }

    /// Open or close a folder, and make it where the next new note goes.
    pub fn toggle_folder(&self, path: &str) {
        let imp = self.imp();
        {
            let mut expanded = imp.expanded.borrow_mut();
            if !expanded.remove(path) {
                expanded.insert(path.to_string());
            }
        }
        imp.target_folder.replace(Some(path.to_string()));
        self.refresh_notes();
    }

    /// Open every folder on the way to this one, so a note revealed by a
    /// search or a link is visible in the tree behind it.
    fn expand_to(&self, id: &NoteId) {
        let Some(folder) = id.folder() else {
            return;
        };
        let mut expanded = self.imp().expanded.borrow_mut();
        let mut path = String::new();
        for segment in folder.split('/') {
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(segment);
            expanded.insert(path.clone());
        }
    }

    pub fn sort(&self) -> Sort {
        self.imp().sort.get()
    }

    pub fn set_sort(&self, sort: Sort) {
        self.imp().sort.set(sort);
        self.refresh_notes();
        if let Some(window) = self.window() {
            window.select_note(self.imp().open.borrow().as_ref());
        }
    }

    /// Filter the sidebar by what was typed into its search entry.
    pub fn set_query(&self, query: &str) {
        if self.imp().query.borrow().as_str() == query {
            return;
        }
        self.imp().query.replace(query.to_string());
        self.refresh_notes();
        if let Some(window) = self.window() {
            window.select_note(self.imp().open.borrow().as_ref());
        }
    }

    /// Every tag in the vault, with its note count.
    pub fn tags(&self) -> Vec<(String, usize)> {
        self.imp().index.borrow().tags()
    }

    /// The notes linking to the open one, each with the line it was linked
    /// from.
    pub fn backlinks_of_open_note(&self) -> Vec<(NoteId, String)> {
        let imp = self.imp();
        let Some(id) = imp.open.borrow().clone() else {
            return Vec::new();
        };
        imp.index
            .borrow()
            .backlinks(&id)
            .iter()
            .map(|backlink| (backlink.from.clone(), backlink.context.clone()))
            .collect()
    }

    fn refresh_notes(&self) {
        let Some(window) = self.window() else {
            return;
        };
        let filter = self.imp().filter.borrow().clone();

        if self.is_searching() {
            let results = self.search_results();
            window.set_results(&results);
            window.set_result_count(Some(results.len()));
        } else {
            window.set_rows(&self.sidebar_rows());
            window.set_result_count(None);
        }
        window.set_tags(&self.tags());
        window.set_active_tag(filter.as_deref());
    }

    /// Show only notes carrying `tag`, or all of them.
    ///
    /// A tag that no longer exists — the last note carrying it was retagged —
    /// clears the filter rather than showing an empty list with no way out.
    pub fn filter_by_tag(&self, tag: Option<&str>) {
        let wanted = tag.map(|tag| tag.trim_start_matches('#').to_lowercase());
        let known = wanted
            .as_ref()
            .is_some_and(|tag| !self.imp().index.borrow().notes_tagged(tag).is_empty());

        self.imp().filter.replace(if known { wanted } else { None });
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
        self.imp().open.borrow().clone()
    }

    pub fn open_note(&self, id: &NoteId) {
        if self.imp().open.borrow().as_ref() == Some(id) {
            return;
        }
        // Switching notes flushes the one being left, rather than waiting for
        // the tick and racing it.
        self.flush_open_note();
        self.save_now();
        self.load_note(id);
    }

    fn load_note(&self, id: &NoteId) {
        let imp = self.imp();
        let Some(vault) = imp.vault.borrow().clone() else {
            return;
        };
        match vault.read(id) {
            Ok(note) => {
                imp.on_disk.replace(Some(note.to_text()));
                imp.open.replace(Some(id.clone()));
                imp.buffer.replace(Some(note));
                imp.config.borrow_mut().last_note = Some(id.as_str().to_string());
                imp.target_folder
                    .replace(Some(id.folder().unwrap_or("").to_string()));
                // Opening a note the tree cannot show — from a link, a search
                // result, or the last session — reveals it rather than leaving
                // the highlight inside a folder that is shut.
                self.expand_to(id);
            }
            Err(error) => {
                if let Some(window) = self.window() {
                    window.toast(&format!("Could not open {error}"));
                }
                imp.open.replace(None);
                imp.buffer.replace(None);
                imp.on_disk.replace(None);
            }
        }
        self.show_open_note();
    }

    fn show_open_note(&self) {
        let Some(window) = self.window() else {
            return;
        };
        let imp = self.imp();
        let open = imp.open.borrow().clone();
        let buffer = imp.buffer.borrow().clone();
        self.refresh_backlinks();
        match (open, buffer) {
            // The editor holds the *file*, frontmatter included: the design
            // styles that block in place, and a note whose metadata is only
            // reachable through some other pane is a note with a hidden half.
            // Round-tripping it costs nothing — an untouched block is written
            // back byte for byte.
            (Some(id), Some(note)) => window.show_note(Some((&id, &note.to_text()))),
            _ => window.show_note(None),
        }
    }

    fn restore_last_note(&self) {
        let last = self.imp().config.borrow().last_note.clone();
        let Some(last) = last else {
            return;
        };
        let id = NoteId::from_relative(last);
        if self.imp().index.borrow().contains(&id) {
            self.load_note(&id);
        }
    }

    /// The editor's text changed.
    pub fn note_edited(&self) {
        self.imp().dirty.set(true);
    }

    /// Copy the editor's text into the open note. Does not write to disk.
    ///
    /// Kept separate from [`Self::save_now`] because the two have different
    /// callers: every save flushes first, but the window also flushes when it
    /// is about to hand control somewhere the editor may go away.
    pub fn flush_open_note(&self) {
        let Some(window) = self.window() else {
            return;
        };
        let Some(body) = window.editor_body() else {
            return;
        };
        let imp = self.imp();
        let mut buffer = imp.buffer.borrow_mut();
        let Some(note) = buffer.as_mut() else {
            return;
        };
        // The editor holds the whole file, so the frontmatter is re-split out
        // of it rather than carried alongside.
        let edited = Note::from_text(note.id.clone(), &body);
        if edited != *note {
            *note = edited;
            imp.dirty.set(true);
        }
    }

    /// Write the open note if it has unsaved changes.
    pub fn save_now(&self) {
        let imp = self.imp();
        if !imp.dirty.get() {
            return;
        }
        let (Some(vault), Some(note)) = (imp.vault.borrow().clone(), imp.buffer.borrow().clone())
        else {
            imp.dirty.set(false);
            return;
        };

        match vault.write(&note) {
            Ok(()) => {
                imp.dirty.set(false);
                imp.on_disk.replace(Some(note.to_text()));
                imp.index.borrow_mut().update(&note);
                if let Some(window) = self.window() {
                    window.set_save_error(None);
                }
                // Rebuilding the list clears the highlight, so put it back.
                self.refresh_notes();
                self.refresh_backlinks();
                if let Some(window) = self.window() {
                    window.select_note(imp.open.borrow().as_ref());
                }
            }
            Err(error) => self.report_save_failure(&error),
        }
    }

    /// Saving is broken. Say so and leave the note dirty so the next tick tries
    /// again — a note that failed to save must not be quietly forgotten.
    fn report_save_failure(&self, error: &VaultError) {
        if let Some(window) = self.window() {
            window.set_save_error(Some(&format!("Not saving: {error}")));
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
        let index = self.imp().index.borrow();
        let open = self.imp().open.borrow().clone();
        search::by_title(&index, query, 16)
            .into_iter()
            // Linking a note to itself is never what was meant.
            .filter(|matched| Some(&matched.id) != open.as_ref())
            .map(|matched| matched.id.title().to_string())
            .collect()
    }

    /// Follow a `[[link]]`.
    ///
    /// A target with nothing behind it is offered as a note to write, because
    /// writing the link before the note is the normal order of things.
    pub fn follow_link(&self, target: &str) {
        let open = self.imp().open.borrow().clone();
        let resolution = self.imp().index.borrow().resolve(target, open.as_ref());
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
        window.set_backlinks(&self.backlinks_of_open_note());

        let imp = self.imp();
        let open = imp.open.borrow().clone();
        let note = imp.buffer.borrow().clone();
        let (tags, words, created, updated) = match &note {
            Some(note) => (
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
            ),
            None => (Vec::new(), 0, None, None),
        };
        window.set_details(open.as_ref(), &tags, words, created, updated);
    }

    /// Answer a palette query.
    pub fn search(&self, query: &str, mode: crate::ui::Mode) -> Vec<crate::ui::Hit> {
        let index = self.imp().index.borrow();
        match mode {
            crate::ui::Mode::Title => search::by_title(&index, query, 30)
                .into_iter()
                .map(|matched| crate::ui::Hit {
                    id: matched.id.as_str().to_string(),
                    title: matched.id.title().to_string(),
                    // The folder, since two notes can share a title and the
                    // path is the only thing telling them apart.
                    detail: matched.id.folder().unwrap_or("").to_string(),
                    highlight: None,
                })
                .collect(),
            // Hybrid: BM25 over the words, vectors over the meaning, fused.
            // The vector half is whatever is already known about this query —
            // on the first keystroke that is nothing, so the first answer is
            // lexical and the fused one replaces it a moment later.
            crate::ui::Mode::Text => {
                let lexical = self.imp().lexical.borrow();
                let vectors = self.imp().vectors.borrow();
                let embedded = self.imp().query_vector.borrow();
                let semantic = embedded
                    .as_ref()
                    .filter(|(embedded, _)| embedded == query)
                    .map(|(_, vector)| (&*vectors, vector.as_slice()));
                if semantic.is_none() {
                    self.embed_query(query);
                }

                search::hybrid(&index, &lexical, semantic, query, 30)
                    .into_iter()
                    .map(|hit| crate::ui::Hit {
                        id: hit.id.as_str().to_string(),
                        title: hit.id.title().to_string(),
                        // Underlined where the words are, and not underlined at
                        // all when the vectors found it and the words are not
                        // there — which is the honest rendering of a hit that
                        // matched on meaning.
                        highlight: search::highlight_of(&hit.snippet, query),
                        detail: hit.snippet,
                    })
                    .collect()
            }
        }
    }

    // ---- attachments ----

    /// Copy dropped files into the vault and embed them.
    pub fn attach_files(&self, paths: &[String]) {
        let Some(vault) = self.imp().vault.borrow().clone() else {
            return;
        };
        let Some(window) = self.window() else {
            return;
        };

        let mut names = Vec::new();
        let mut failures = 0usize;
        for path in paths {
            match vault.add_attachment(Path::new(path)) {
                Ok(name) => names.push(name),
                Err(_) => failures += 1,
            }
        }

        if !names.is_empty() {
            window.insert_embeds(&names);
            // The embed only points at a file; the save tick writes the note.
            self.flush_open_note();
        }
        if failures > 0 {
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
        let Some(vault) = self.imp().vault.borrow().clone() else {
            return;
        };

        let referenced = self.imp().index.borrow().referenced_attachments();
        let directory = vault.root().join(crate::model::vault::ATTACHMENTS_DIR);
        let mut unused: Vec<String> = std::fs::read_dir(&directory)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| !name.starts_with('.') && !referenced.contains(name))
            .collect();
        unused.sort();

        window.show_unused_attachments(&unused);
    }

    // ---- creating, renaming, deleting ----

    pub fn create_note(&self, title: &str) {
        self.create_note_in(&self.current_folder(), title);
    }

    /// Where a new note or folder goes: whichever folder was last chosen in the
    /// sidebar, and otherwise beside the open note, so working inside a folder
    /// keeps you there.
    fn current_folder(&self) -> String {
        let imp = self.imp();
        let target = imp.target_folder.borrow().clone();
        target
            .or_else(|| {
                imp.open
                    .borrow()
                    .as_ref()
                    .and_then(|id| id.folder().map(str::to_string))
            })
            .unwrap_or_default()
    }

    /// Write a new note in a named folder.
    pub fn create_note_in(&self, folder: &str, title: &str) {
        let imp = self.imp();
        let Some(vault) = imp.vault.borrow().clone() else {
            return;
        };
        self.flush_open_note();
        self.save_now();

        let id = self.unique_id(&vault, Some(folder), title);

        match vault.create(&id, "") {
            Ok(note) => {
                imp.index.borrow_mut().update(&note);
                self.refresh_notes();
                self.load_note(&id);
            }
            Err(error) => {
                if let Some(window) = self.window() {
                    window.toast(&format!("Could not create the note: {error}"));
                }
            }
        }
    }

    /// A free id for `title`, suffixed if that name is taken.
    fn unique_id(&self, vault: &Vault, folder: Option<&str>, title: &str) -> NoteId {
        let build = |name: &str| match folder {
            Some(folder) if !folder.is_empty() => {
                NoteId::from_relative(format!("{folder}/{name}.md"))
            }
            _ => NoteId::from_relative(format!("{name}.md")),
        };
        let mut id = build(title);
        let mut attempt = 2;
        while vault.path_of(&id).exists() {
            id = build(&format!("{title} {attempt}"));
            attempt += 1;
        }
        id
    }

    /// Rename the open note and repoint every link that pointed at it.
    pub fn rename_note(&self, title: &str) {
        let imp = self.imp();
        let Some(vault) = imp.vault.borrow().clone() else {
            return;
        };
        let Some(from) = imp.open.borrow().clone() else {
            return;
        };
        if from.title() == title {
            return;
        }

        self.flush_open_note();
        self.save_now();

        let to = match from.folder() {
            Some(folder) => NoteId::from_relative(format!("{folder}/{title}.md")),
            None => NoteId::from_relative(format!("{title}.md")),
        };

        // Every note pointing here has to be rewritten, so gather them before
        // the rename makes the index forget who they were.
        let inbound: Vec<NoteId> = imp
            .index
            .borrow()
            .backlinks(&from)
            .iter()
            .map(|backlink| backlink.from.clone())
            .collect();

        if let Err(error) = vault.rename(&from, &to) {
            if let Some(window) = self.window() {
                window.toast(&format!("Could not rename: {error}"));
            }
            return;
        }
        imp.index.borrow_mut().rename(&from, &to);

        let mut rewritten = 0usize;
        for id in inbound {
            let Ok(mut note) = vault.read(&id) else {
                continue;
            };
            let Some(body) = markdown::rewrite_target(&note.body, from.title(), to.title()) else {
                continue;
            };
            note.body = body;
            if vault.write(&note).is_ok() {
                imp.index.borrow_mut().update(&note);
                rewritten += 1;
            }
        }

        imp.open.replace(Some(to.clone()));
        if let Some(note) = imp.buffer.borrow_mut().as_mut() {
            note.id = to.clone();
        }
        imp.config.borrow_mut().last_note = Some(to.as_str().to_string());

        self.refresh_notes();
        self.show_open_note();

        if let Some(window) = self.window() {
            match rewritten {
                0 => window.toast("Renamed"),
                1 => window.toast("Renamed, and updated 1 link"),
                count => window.toast(&format!("Renamed, and updated {count} links")),
            }
        }
    }

    pub fn delete_open_note(&self) {
        let imp = self.imp();
        let Some(vault) = imp.vault.borrow().clone() else {
            return;
        };
        let Some(id) = imp.open.borrow().clone() else {
            return;
        };

        // Drop the pending write first, or the tick recreates the file.
        imp.dirty.set(false);
        imp.buffer.replace(None);
        imp.on_disk.replace(None);
        imp.open.replace(None);

        if let Err(error) = vault.delete(&id) {
            if let Some(window) = self.window() {
                window.toast(&format!("Could not delete: {error}"));
            }
            return;
        }
        imp.index.borrow_mut().remove(&id);
        imp.config.borrow_mut().last_note = None;

        self.refresh_notes();
        self.show_open_note();
        if let Some(window) = self.window() {
            window.toast(&format!("Deleted “{}”", id.title()));
        }
    }

    // ---- folders ----

    /// The folders in the vault, for the dialogs that ask which one.
    pub fn folders(&self) -> Vec<String> {
        self.imp()
            .vault
            .borrow()
            .as_ref()
            .map(Vault::folders)
            .unwrap_or_default()
    }

    /// Make a folder where a new note would go.
    pub fn create_folder_here(&self, name: &str) {
        self.create_folder(&self.current_folder(), name);
    }

    pub fn create_folder(&self, parent: &str, name: &str) {
        let Some(vault) = self.imp().vault.borrow().clone() else {
            return;
        };
        let path = join(parent, name);
        match vault.create_folder(&path) {
            Ok(()) => {
                // Opened, and opened all the way down to it: a folder made and
                // then not shown is a folder you go looking for in Files.
                let mut expanded = self.imp().expanded.borrow_mut();
                for ancestor in ancestors(&path) {
                    expanded.insert(ancestor);
                }
                drop(expanded);
                self.imp().target_folder.replace(Some(path));
                self.refresh_notes();
            }
            Err(error) => self.complain(&format!("Could not create the folder: {error}")),
        }
    }

    /// Rename a folder in place, keeping everything under it.
    pub fn rename_folder(&self, path: &str, name: &str) {
        let parent = path
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .unwrap_or("");
        self.relocate_folder(path, &join(parent, name), "Renamed");
    }

    pub fn delete_folder(&self, path: &str) {
        let Some(vault) = self.imp().vault.borrow().clone() else {
            return;
        };
        match vault.delete_folder(path) {
            Ok(()) => {
                self.imp().expanded.borrow_mut().remove(path);
                self.forget_target(path);
                self.refresh_notes();
                if let Some(window) = self.window() {
                    window.toast(&format!("Deleted “{path}”"));
                }
            }
            Err(error) => self.complain(&format!("Could not delete the folder: {error}")),
        }
    }

    /// Something was dragged onto a folder. Move it, or say why not.
    pub fn move_dropped(&self, payload: &str, destination: &str) {
        match crate::ui::sidebar::dragged(payload) {
            Some(Dragged::Note(id)) => self.move_note(&id, destination),
            Some(Dragged::Folder(path)) => {
                let name = path.rsplit('/').next().unwrap_or(&path).to_string();
                self.relocate_folder(&path, &join(destination, &name), "Moved");
            }
            // Something dragged in from another app. Notes are files, so this
            // could one day mean "import", but silently doing nothing is
            // better than guessing at it.
            None => {}
        }
    }

    /// Move one note into a folder, keeping its title.
    ///
    /// Inbound links are left alone deliberately: they resolve by title, and
    /// the title has not changed. A move is the one rearrangement of the vault
    /// that costs nothing anywhere else — which is the point of folders being
    /// organisational rather than namespaces.
    pub fn move_note(&self, id: &NoteId, destination: &str) {
        let imp = self.imp();
        let Some(vault) = imp.vault.borrow().clone() else {
            return;
        };
        if id.folder().unwrap_or("") == destination {
            return;
        }
        let to = NoteId::from_relative(join(destination, &format!("{}.md", id.title())));

        // The note may be the open one with unsaved edits, and the file is
        // about to move out from under the save tick.
        let open = imp.open.borrow().clone();
        if open.as_ref() == Some(id) {
            self.flush_open_note();
            self.save_now();
        }

        if let Err(error) = vault.rename(id, &to) {
            self.complain(&format!("Could not move: {error}"));
            return;
        }
        imp.index.borrow_mut().rename(id, &to);

        if open.as_ref() == Some(id) {
            imp.open.replace(Some(to.clone()));
            if let Some(note) = imp.buffer.borrow_mut().as_mut() {
                note.id = to.clone();
            }
            imp.config.borrow_mut().last_note = Some(to.as_str().to_string());
        }
        self.expand_to(&to);
        self.refresh_notes();
        self.show_open_note();
        if let Some(window) = self.window() {
            window.toast(&match destination {
                "" => format!("Moved “{}” to the vault root", to.title()),
                folder => format!("Moved “{}” to {folder}", to.title()),
            });
        }
    }

    /// Move or rename a folder, and take the app's own state with it.
    ///
    /// The index is rebuilt rather than patched: every note under the folder
    /// changes id at once, and walking a subtree of the index to rewrite ids
    /// is exactly the kind of bookkeeping a rescan does correctly for free.
    fn relocate_folder(&self, from: &str, to: &str, verb: &str) {
        let imp = self.imp();
        let Some(vault) = imp.vault.borrow().clone() else {
            return;
        };
        if from == to {
            return;
        }
        self.flush_open_note();
        self.save_now();

        if let Err(error) = vault.move_folder(from, to) {
            self.complain(&format!("Could not move the folder: {error}"));
            return;
        }

        // Anything that named a path under the old folder now names nothing.
        let moved = |path: &str| -> Option<String> {
            if path == from {
                Some(to.to_string())
            } else {
                path.strip_prefix(&format!("{from}/"))
                    .map(|rest| format!("{to}/{rest}"))
            }
        };
        let expanded: std::collections::BTreeSet<String> = imp
            .expanded
            .borrow()
            .iter()
            .map(|path| moved(path).unwrap_or_else(|| path.clone()))
            .collect();
        imp.expanded.replace(expanded);
        for ancestor in ancestors(to) {
            imp.expanded.borrow_mut().insert(ancestor);
        }
        // Taken out of the RefCell before the `if let`, not inside its
        // scrutinee: the borrow there lives for the whole body and the
        // `replace` would panic on it.
        let target = imp.target_folder.borrow().clone();
        if let Some(target) = target {
            imp.target_folder
                .replace(Some(moved(&target).unwrap_or(target)));
        }

        let reopen = imp
            .open
            .borrow()
            .clone()
            .and_then(|id| moved(id.as_str()).map(NoteId::from_relative));

        self.rescan();
        if let Some(id) = reopen {
            imp.open.replace(Some(id.clone()));
            imp.config.borrow_mut().last_note = Some(id.as_str().to_string());
            self.load_note(&id);
        }
        self.refresh_notes();
        if let Some(window) = self.window() {
            window.select_note(imp.open.borrow().as_ref());
            window.toast(&format!("{verb} “{to}”"));
        }
    }

    /// Drop a remembered target folder that has just stopped existing.
    fn forget_target(&self, path: &str) {
        let imp = self.imp();
        let stale = imp
            .target_folder
            .borrow()
            .as_deref()
            .is_some_and(|target| tree::is_within(path, target));
        if stale {
            imp.target_folder.replace(None);
        }
    }

    fn complain(&self, message: &str) {
        if let Some(window) = self.window() {
            window.toast(message);
        }
    }
}

/// `parent` and `name` as one vault-relative path. An empty parent is the
/// vault root, where a leading `/` would name something outside it.
fn join(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

/// `a/b/c` yields `a`, `a/b`, `a/b/c`.
fn ancestors(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for segment in path.split('/').filter(|segment| !segment.is_empty()) {
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(segment);
        out.push(current.clone());
    }
    out
}
