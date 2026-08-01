//! The note editor: one `GtkTextView` holding one note's body, always styled.
//!
//! The editor never writes a file. It reports that the text changed and the
//! application decides when that reaches disk, so there is exactly one place a
//! note can be lost.
//!
//! # Re-styling on a keystroke
//!
//! A full re-scan on every keypress is fine for a sticky note and not for a
//! 3,000-word page. So the editor caches the [`LineState`] each line begins in
//! and, for an edit that stays inside one line, re-scans only that line and the
//! one above it — above too, because a table header is only a header if the
//! line under it is a delimiter row, so editing a line can change the meaning
//! of its predecessor.
//!
//! Anything else — a newline, a paste, a deletion spanning lines, or an edit
//! whose outgoing `LineState` differs from the cached one (a fence opened, a
//! frontmatter delimiter touched) — escalates to a full re-scan. Getting the
//! escalation wrong shows stale styling that only reloading fixes, so the rule
//! is deliberately blunt, and `tests/widgets.rs` asserts the incremental result
//! is identical to the full one.

use std::cell::{Cell, RefCell};
use std::sync::OnceLock;

use gtk::glib::subclass::Signal;
use gtk::glib::{self, clone};
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::model::markdown::{self, Edit, Format, Marker, Parsed, Span};
use crate::ui::{attachments, highlight, LinkPopover};
use std::path::PathBuf;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Editor {
        pub view: RefCell<Option<gtk::TextView>>,
        /// Set while the editor is loading a note into the buffer, so the
        /// change handler does not report that write back as the user typing.
        /// Without it, opening a note marks it dirty and every note you look at
        /// gets rewritten.
        pub loading: Cell<bool>,
        /// The most recent scan of the whole buffer. Kept because revealing
        /// markers on a cursor move needs the marker list, and re-scanning the
        /// document to move a caret would defeat the point of the cache.
        pub parsed: RefCell<Parsed>,
        /// Line count at the last scan, to tell an in-line edit from one that
        /// added or removed a line.
        pub lines: Cell<i32>,
        /// Character count at the last scan. An edit moves every offset below
        /// it by the difference, and the cache is in absolute offsets.
        pub chars: Cell<i32>,
        /// The cursor offset the current reveal was computed for, so a caret
        /// move that stays inside the same construct does no work. `None` when
        /// nothing has been revealed yet.
        pub revealed: Cell<Option<usize>>,
        /// Reading mode: no caret, no editing, and no syntax anywhere.
        pub reading: Cell<bool>,
        /// Whether a note is open at all. Both this and `reading` have to be
        /// right for the view to accept typing, and each is set by a different
        /// part of the app, so neither may write the view's `editable`
        /// directly.
        pub has_note: Cell<bool>,
        /// How many times the whole buffer has been re-scanned. Only read by
        /// tests, which would otherwise pass just as happily against an editor
        /// that escalated every keystroke and made the cache pointless.
        pub full_rescans: Cell<u32>,
        /// Kept alive for the widget's lifetime; dropping it unsubscribes from
        /// the style manager, which outlives this widget.
        pub scheme_handlers: RefCell<Vec<glib::SignalHandlerId>>,
        pub popover: RefCell<Option<LinkPopover>>,
        /// Character offset just past the `[[` the popover is completing, so
        /// accepting a candidate knows what to replace.
        pub completing_from: Cell<Option<usize>>,
        /// The vault root, so an `![[embed]]` can be found on disk. The editor
        /// knows where files live; it does not know what a vault is.
        pub root: RefCell<Option<PathBuf>>,
        /// The overlay slots the embeds are drawn in. An overlay cannot be
        /// removed from a `GtkTextView` at all, so they are kept here and
        /// refilled, and the spare ones hidden.
        pub embeds: RefCell<Vec<gtk::Widget>>,
        /// The query last asked about. A keystroke moves the cursor *and*
        /// changes the text, so without this every character searches the
        /// index twice.
        pub last_query: RefCell<Option<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Editor {
        const NAME: &'static str = "BrainEditor";
        type Type = super::Editor;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for Editor {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }

        fn dispose(&self) {
            // The popover is parented to this widget, so it has to be
            // unparented here or GTK warns on finalize.
            if let Some(popover) = self.popover.take() {
                popover.destroy();
            }
            // The style manager outlives this widget, so its handlers have to
            // go with the widget or they fire against a dead object.
            let manager = adw::StyleManager::default();
            for handler in self.scheme_handlers.take() {
                manager.disconnect(handler);
            }
            // A Widget subclass must unparent its children, or GTK warns on
            // finalize and the child leaks.
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    // The user changed the text. Carries nothing: the
                    // application reads the body back when it is ready to.
                    Signal::builder("body-changed").build(),
                    // The user followed a `[[link]]`. Carries the target as
                    // written; resolving it against real notes is the index's
                    // job, not the editor's.
                    Signal::builder("link-activated")
                        .param_types([String::static_type()])
                        .build(),
                    // The text between an open `[[` and the cursor changed.
                    // The handler is expected to call `set_link_candidates`
                    // before returning — emission is synchronous, so the
                    // popover is updated in the same turn as the keystroke.
                    Signal::builder("link-query")
                        .param_types([String::static_type()])
                        .build(),
                    // The user followed a `#tag`, without its hash.
                    Signal::builder("tag-activated")
                        .param_types([String::static_type()])
                        .build(),
                    // Files were dropped on the editor. Carries absolute paths;
                    // copying them into the vault is the application's job.
                    Signal::builder("files-dropped")
                        .param_types([Vec::<String>::static_type()])
                        .build(),
                    // An image was pasted. Carries the path of a temporary PNG
                    // the application should take ownership of.
                    Signal::builder("image-pasted")
                        .param_types([String::static_type()])
                        .build(),
                ]
            })
        }
    }

    impl WidgetImpl for Editor {}
}

glib::wrapper! {
    pub struct Editor(ObjectSubclass<imp::Editor>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    pub fn new() -> Self {
        glib::Object::new()
    }

    fn build(&self) {
        let view = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .top_margin(24)
            .bottom_margin(96) // room to type past the bottom of the window
            .left_margin(24)
            .right_margin(24)
            .pixels_below_lines(4)
            .accepts_tab(false) // Tab moves focus; notes are not code
            .build();
        view.add_css_class("editor");

        let buffer = view.buffer();
        highlight::install(&view);

        buffer.connect_changed(clone!(
            #[weak(rename_to = editor)]
            self,
            move |_| {
                if editor.imp().loading.get() {
                    return;
                }
                editor.restyle();
                editor.update_completion();
                editor.emit_by_name::<()>("body-changed", &[]);
            }
        ));

        // Moving the caret out of a construct is what hides its syntax again,
        // and moving into the next one is what shows that.
        buffer.connect_notify_local(
            Some("cursor-position"),
            clone!(
                #[weak(rename_to = editor)]
                self,
                move |_, _| {
                    editor.update_revealed(false);
                    // Clicking or arrowing out of the brackets abandons the
                    // completion, rather than leaving a list pointing at text
                    // the cursor has left.
                    if editor.imp().completing_from.get().is_some() {
                        editor.update_completion();
                    }
                }
            ),
        );

        // The tag colours are not CSS, so they do not follow the theme on their
        // own. Both the scheme and the accent can change while the app runs.
        let manager = adw::StyleManager::default();
        let mut handlers = Vec::new();
        for property in ["dark", "accent-color"] {
            handlers.push(manager.connect_notify_local(
                Some(property),
                clone!(
                    #[weak(rename_to = editor)]
                    self,
                    move |_, _| highlight::refresh_colours(&editor.view())
                ),
            ));
        }
        self.imp().scheme_handlers.replace(handlers);

        // Prose set to the full width of a maximised window is unreadable —
        // the eye loses the start of the next line. The clamp holds the
        // measure at something like 80 characters and centres it, which is
        // what every GNOME app that shows a document does.
        let clamp = adw::Clamp::builder()
            .maximum_size(760)
            .tightening_threshold(680)
            .child(&view)
            .build();

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&clamp)
            .build();
        scroller.set_parent(self);

        // The scroller expanding is not enough: this widget is what the box
        // above measures, and without expanding itself it is allocated its
        // minimum height — an editor about one line tall.
        self.set_vexpand(true);
        self.set_hexpand(true);

        self.imp().view.replace(Some(view.clone()));

        let popover = LinkPopover::new(self);
        popover.connect_closure(
            "chosen",
            false,
            glib::closure_local!(
                #[weak(rename_to = editor)]
                self,
                move |_: LinkPopover, chosen: String| editor.accept_completion(&chosen)
            ),
        );
        self.imp().popover.replace(Some(popover));

        self.install_link_gestures(&view);
        self.install_drop_target(&view);

        // The foreground colour the palette is derived from is only resolved
        // once the widget has a style, which is after it is realised.
        view.connect_realize(highlight::refresh_colours);

        // Overlays sit at buffer coordinates, which are only known once the
        // text has been laid out. The adjustment's "changed" fires whenever
        // the laid-out size moves — a re-wrap, a resize, an edit — which is
        // exactly when an overlay is in the wrong place.
        if let Some(adjustment) = view.vadjustment() {
            adjustment.connect_changed(clone!(
                #[weak(rename_to = editor)]
                self,
                move |_| editor.reposition_embeds()
            ));
        }
    }

    fn install_drop_target(&self, view: &gtk::TextView) {
        let drop = gtk::DropTarget::new(
            gtk::gdk::FileList::static_type(),
            gtk::gdk::DragAction::COPY,
        );
        drop.connect_drop(clone!(
            #[weak(rename_to = editor)]
            self,
            #[upgrade_or]
            false,
            move |_, value, _, _| {
                let Ok(files) = value.get::<gtk::gdk::FileList>() else {
                    return false;
                };
                let paths: Vec<String> = files
                    .files()
                    .into_iter()
                    .filter_map(|file| file.path())
                    .map(|path| path.to_string_lossy().to_string())
                    .collect();
                if paths.is_empty() {
                    return false;
                }
                editor.emit_by_name::<()>("files-dropped", &[&paths]);
                true
            }
        ));
        view.add_controller(drop);
    }

    /// Take an image off the clipboard, if there is one.
    ///
    /// Reports whether it handled the paste: text pastes must fall through to
    /// the text view, which knows far more about pasting text than this does.
    fn paste_image(&self) -> bool {
        let Some(display) = gtk::gdk::Display::default() else {
            return false;
        };
        let clipboard = display.clipboard();
        if !clipboard
            .formats()
            .contains_type(gtk::gdk::Texture::static_type())
        {
            return false;
        }

        glib::spawn_future_local(clone!(
            #[weak(rename_to = editor)]
            self,
            async move {
                let Ok(texture) = clipboard.read_texture_future().await else {
                    return;
                };
                let Some(texture) = texture else {
                    return;
                };
                // Written where the application can pick it up and copy it in;
                // the editor does not know where the vault is allowed to write.
                let path =
                    std::env::temp_dir().join(format!("brain-paste-{}.png", glib::real_time()));
                if texture.save_to_png(&path).is_err() {
                    return;
                }
                editor.emit_by_name::<()>("image-pasted", &[&path.to_string_lossy().to_string()]);
            }
        ));
        true
    }

    /// The vault root, for finding embedded files.
    pub fn set_vault_root(&self, root: Option<PathBuf>) {
        self.imp().root.replace(root);
        self.refresh_embeds();
    }

    /// Insert `![[name]]` at the cursor, on a line of its own.
    pub fn insert_embed(&self, name: &str) {
        let buffer = self.view().buffer();
        let mut iter = buffer.iter_at_mark(&buffer.get_insert());
        // An embed shares a line with nothing: the picture is drawn beneath it
        // and prose either side would be split by the gap.
        let prefix = if iter.starts_line() { "" } else { "\n" };
        buffer.insert(&mut iter, &format!("{prefix}![[{name}]]\n"));
    }

    fn refresh_embeds(&self) {
        let imp = self.imp();
        let view = self.view();
        let previous = imp.embeds.take();
        let root = imp.root.borrow().clone();
        let widgets = attachments::refresh(&view, &imp.parsed.borrow(), root.as_deref(), previous);
        let placed = widgets.iter().any(|slot| slot.get_visible());
        imp.embeds.replace(widgets);

        // `iter_location` reports zeros until the view has been laid out, so a
        // position taken during the restyle can be wrong. One pass on the idle
        // puts them right once the layout has run.
        if placed {
            glib::idle_add_local_once(clone!(
                #[weak(rename_to = editor)]
                self,
                move || editor.reposition_embeds()
            ));
        }
    }

    fn reposition_embeds(&self) {
        let imp = self.imp();
        if imp.embeds.borrow().is_empty() {
            return;
        }
        attachments::reposition(&self.view(), &imp.parsed.borrow(), &imp.embeds.borrow());
    }

    fn install_link_gestures(&self, view: &gtk::TextView) {
        // Ctrl+Click follows a link. Plain click does not: a note is a thing
        // you edit, and clicking to place the cursor must never navigate away.
        let click = gtk::GestureClick::new();
        click.connect_released(clone!(
            #[weak(rename_to = editor)]
            self,
            #[weak]
            view,
            move |gesture, _, x, y| {
                // In reading mode there is no cursor to place, so the reason
                // for the modifier is gone and a plain click follows the link.
                let modified = gesture
                    .current_event_state()
                    .contains(gtk::gdk::ModifierType::CONTROL_MASK);
                if !modified && !editor.is_reading() {
                    return;
                }
                let (bx, by) =
                    view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
                let Some(iter) = view.iter_at_location(bx, by) else {
                    return;
                };
                editor.follow_link_at(iter.offset() as usize);
            }
        ));
        view.add_controller(click);

        // A pointing hand over a link, so it looks like one before you commit
        // to the modifier.
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(clone!(
            #[weak(rename_to = editor)]
            self,
            #[weak]
            view,
            move |_, x, y| {
                let (bx, by) =
                    view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
                let over = view.iter_at_location(bx, by).is_some_and(|iter| {
                    let offset = iter.offset() as usize;
                    editor.link_at(offset).is_some() || editor.tag_at(offset).is_some()
                });
                view.set_cursor_from_name(Some(if over { "pointer" } else { "text" }));
            }
        ));
        view.add_controller(motion);

        // The completion popover takes the arrow keys and Return only while it
        // is open, so typing is never intercepted by a list that is not there.
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(clone!(
            #[weak(rename_to = editor)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, state| editor.on_key(key, state)
        ));
        view.add_controller(keys);
    }

    fn on_key(&self, key: gtk::gdk::Key, state: gtk::gdk::ModifierType) -> glib::Propagation {
        use gtk::gdk::Key;

        // Ctrl+V pastes an image if the clipboard holds one, and otherwise
        // lets the text view do what it always does.
        if key == Key::v
            && state.contains(gtk::gdk::ModifierType::CONTROL_MASK)
            && self.paste_image()
        {
            return glib::Propagation::Stop;
        }

        // Ctrl+Return follows the link under the cursor. Plain Return cannot:
        // it is how you start a new line.
        if key == Key::Return && state.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
            let offset = self.cursor_offset();
            if self.follow_link_at(offset) {
                return glib::Propagation::Stop;
            }
            return glib::Propagation::Proceed;
        }

        let Some(popover) = self.imp().popover.borrow().clone() else {
            return glib::Propagation::Proceed;
        };
        if !popover.is_open() {
            return glib::Propagation::Proceed;
        }

        match key {
            Key::Escape => {
                self.close_completion();
                glib::Propagation::Stop
            }
            Key::Down | Key::Tab => {
                popover.move_selection(1);
                glib::Propagation::Stop
            }
            Key::Up | Key::ISO_Left_Tab => {
                popover.move_selection(-1);
                glib::Propagation::Stop
            }
            Key::Return | Key::KP_Enter => match popover.selected() {
                Some(chosen) => {
                    self.accept_completion(chosen.as_str());
                    glib::Propagation::Stop
                }
                None => glib::Propagation::Proceed,
            },
            _ => glib::Propagation::Proceed,
        }
    }

    /// The `#tag` covering a character offset, if there is one.
    fn tag_at(&self, offset: usize) -> Option<String> {
        let text = self.body();
        markdown::extract(&text)
            .tags
            .into_iter()
            .find(|tag| offset >= tag.start && offset < tag.end)
            .map(|tag| tag.name)
    }

    /// The `[[link]]` covering a character offset, if there is one.
    ///
    /// Reads the scanner's output rather than searching the text, so what the
    /// editor treats as a link is exactly what it styled as one — a `[[…]]`
    /// inside a code fence is not clickable, because it is not a link.
    fn link_at(&self, offset: usize) -> Option<String> {
        let text = self.body();
        markdown::extract(&text)
            .links
            .into_iter()
            .find(|link| offset >= link.start && offset < link.end)
            .map(|link| link.target)
    }

    /// Follow whatever is at an offset — a link or a tag. Reports whether
    /// there was anything.
    fn follow_link_at(&self, offset: usize) -> bool {
        if let Some(target) = self.link_at(offset) {
            self.emit_by_name::<()>("link-activated", &[&target]);
            return true;
        }
        if let Some(tag) = self.tag_at(offset) {
            self.emit_by_name::<()>("tag-activated", &[&tag]);
            return true;
        }
        false
    }

    fn cursor_offset(&self) -> usize {
        let buffer = self.view().buffer();
        buffer.iter_at_mark(&buffer.get_insert()).offset().max(0) as usize
    }

    /// Work out whether the cursor sits inside an unclosed `[[`, and if so ask
    /// for candidates.
    fn update_completion(&self) {
        let text = self.body();
        let chars: Vec<char> = text.chars().collect();
        let cursor = self.cursor_offset().min(chars.len());

        let Some(from) = open_link_start(&chars, cursor) else {
            self.close_completion();
            return;
        };

        let query: String = chars[from..cursor].iter().collect();
        self.imp().completing_from.set(Some(from));
        if self.imp().last_query.borrow().as_deref() == Some(query.as_str()) {
            return;
        }
        self.imp().last_query.replace(Some(query.clone()));
        // Synchronous: the handler calls `set_link_candidates` before this
        // returns, so the popover updates in the same turn as the keystroke.
        self.emit_by_name::<()>("link-query", &[&query]);
    }

    /// Show these candidates for the query most recently reported.
    pub fn set_link_candidates(&self, candidates: &[String]) {
        let imp = self.imp();
        let (Some(popover), Some(_)) = (imp.popover.borrow().clone(), imp.completing_from.get())
        else {
            return;
        };
        popover.show(candidates, &self.cursor_rectangle());
    }

    fn close_completion(&self) {
        self.imp().completing_from.set(None);
        self.imp().last_query.replace(None);
        if let Some(popover) = self.imp().popover.borrow().as_ref() {
            popover.hide();
        }
    }

    /// Replace the partial target with `chosen` and close the brackets.
    fn accept_completion(&self, chosen: &str) {
        let imp = self.imp();
        let Some(from) = imp.completing_from.get() else {
            return;
        };
        let cursor = self.cursor_offset();
        let buffer = self.view().buffer();

        let mut start = buffer.iter_at_offset(from as i32);
        let mut end = buffer.iter_at_offset(cursor as i32);
        buffer.delete(&mut start, &mut end);

        // Close the brackets only if the user has not already typed them,
        // which they will have if they went back to edit an existing link.
        let text = self.body();
        let chars: Vec<char> = text.chars().collect();
        let closed = chars.get(from) == Some(&']') && chars.get(from + 1) == Some(&']');
        let insertion = if closed {
            chosen.to_string()
        } else {
            format!("{chosen}]]")
        };

        let mut at = buffer.iter_at_offset(from as i32);
        buffer.insert(&mut at, &insertion);

        // Land the cursor after the closing brackets, ready to keep writing.
        let after = (from + chosen.chars().count() + 2) as i32;
        buffer.place_cursor(&buffer.iter_at_offset(after.min(buffer.char_count())));
        self.close_completion();
    }

    /// Where to point the popover: just under the cursor.
    fn cursor_rectangle(&self) -> gtk::gdk::Rectangle {
        let view = self.view();
        let buffer = view.buffer();
        let iter = buffer.iter_at_mark(&buffer.get_insert());
        let location = view.iter_location(&iter);
        let (x, y) = view.buffer_to_window_coords(
            gtk::TextWindowType::Widget,
            location.x(),
            location.y() + location.height(),
        );
        gtk::gdk::Rectangle::new(x, y, 1, 1)
    }

    /// Re-scan and re-tag after an edit.
    ///
    /// Takes the cheap path when the edit stayed inside one line and left the
    /// line's outgoing state alone; otherwise re-scans everything.
    fn restyle(&self) {
        let imp = self.imp();
        let buffer = self.view().buffer();

        let lines = buffer.line_count();
        let cursor = self.cursor_line();
        let previously = imp.lines.replace(lines);
        // How much longer the buffer got, which is how far the edit pushed
        // everything below it.
        let shift = (buffer.char_count() - imp.chars.replace(buffer.char_count())) as isize;

        // A changed line count means text moved between lines, and every
        // cached offset below the edit is now wrong.
        if lines != previously {
            self.restyle_all();
            return;
        }

        // Re-scan the line above too: a table header is only a header because
        // of the line beneath it, so editing a line can change what the line
        // above one means.
        let first = cursor.saturating_sub(1).max(0);
        let cached = imp.parsed.borrow().line_states.get(first as usize).copied();
        let cached_list = imp.parsed.borrow().line_lists.get(first as usize).copied();
        let (Some(mut state), Some(mut list)) = (cached, cached_list) else {
            self.restyle_all();
            return;
        };

        let mut scanned = Parsed::default();
        for line in first..=cursor {
            let (text, offset) = self.line_text(&buffer, line);
            let (next_text, _) = self.line_text(&buffer, line + 1);
            let next = (line + 1 < lines).then_some(next_text.as_str());
            let (parsed, outgoing, outgoing_list) =
                markdown::scan_line(&text, offset as usize, state, list, next);
            scanned.spans.extend(parsed.spans);
            scanned.markers.extend(parsed.markers);
            scanned.line_states.push(state);
            scanned.line_lists.push(list);
            state = outgoing;
            list = outgoing_list;
        }

        // The state the next line begins in has to be what the cache says, or
        // everything below this point is styled against the wrong assumption.
        let expected = imp
            .parsed
            .borrow()
            .line_states
            .get((cursor + 1) as usize)
            .copied();
        if let Some(expected) = expected {
            if expected != state {
                self.restyle_all();
                return;
            }
        }
        // The list nesting is part of that state: opening a nested item
        // changes the depth of every item under it.
        let expected_list = imp
            .parsed
            .borrow()
            .line_lists
            .get((cursor + 1) as usize)
            .copied();
        if let Some(expected_list) = expected_list {
            if expected_list != list {
                self.restyle_all();
                return;
            }
        }

        // Splice the re-scanned lines into the cached scan, so the next cursor
        // move has a complete marker list to work from.
        //
        // The cache was made before the edit, so it is in the *old* offsets:
        // the re-scanned lines ended at `to` back then, and everything below
        // has since moved by `shift`. Comparing old offsets against the new
        // `to` would keep a stale span from inside the edit — a deletion
        // shrinks the window out from under it — and would leave every span
        // below the edit pointing `shift` characters wide of its text.
        let from = self.line_offset(&buffer, first);
        let to = self.line_offset(&buffer, cursor + 1);
        let was_to = (to as isize - shift) as usize;
        let moved = |offset: usize| (offset as isize + shift).max(0) as usize;
        {
            let mut parsed = imp.parsed.borrow_mut();
            let below: Vec<_> = parsed
                .spans
                .iter()
                .filter(|span| span.start >= was_to)
                .map(|span| Span {
                    start: moved(span.start),
                    end: moved(span.end),
                    style: span.style,
                })
                .collect();
            let below_markers: Vec<_> = parsed
                .markers
                .iter()
                .filter(|marker| marker.start >= was_to)
                .map(|marker| Marker {
                    start: moved(marker.start),
                    end: moved(marker.end),
                    reveal: (moved(marker.reveal.0), moved(marker.reveal.1)),
                })
                .collect();
            parsed.spans.retain(|span| span.start < from as usize);
            parsed.markers.retain(|marker| marker.start < from as usize);
            parsed.spans.extend(scanned.spans.iter().copied());
            parsed.markers.extend(scanned.markers.iter().copied());
            parsed.spans.extend(below);
            parsed.markers.extend(below_markers);
            for (index, line_state) in scanned.line_states.iter().enumerate() {
                if let Some(slot) = parsed.line_states.get_mut(first as usize + index) {
                    *slot = *line_state;
                }
            }
            for (index, levels) in scanned.line_lists.iter().enumerate() {
                if let Some(slot) = parsed.line_lists.get_mut(first as usize + index) {
                    *slot = *levels;
                }
            }
        }

        highlight::clear(&buffer, from, to);
        highlight::apply(&buffer, &scanned);
        imp.revealed.set(None);
        self.update_revealed(true);
        self.refresh_embeds();
    }

    /// Re-scan the whole buffer.
    fn restyle_all(&self) {
        self.imp()
            .full_rescans
            .set(self.imp().full_rescans.get() + 1);
        let buffer = self.view().buffer();
        let text = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), true)
            .to_string();
        let parsed = markdown::parse(&text);

        highlight::clear(&buffer, 0, buffer.char_count());
        highlight::apply(&buffer, &parsed);

        let imp = self.imp();
        imp.parsed.replace(parsed);
        imp.lines.set(buffer.line_count());
        imp.chars.set(buffer.char_count());
        imp.revealed.set(None);
        self.update_revealed(true);
        self.refresh_embeds();
    }

    /// Show the syntax of the construct the caret is in, and hide it
    /// everywhere else.
    ///
    /// Skipped when the caret has not moved since the last reveal, because
    /// this runs on every keystroke as well as every arrow key.
    fn update_revealed(&self, force: bool) {
        let imp = self.imp();
        let cursor = self.cursor_offset();
        if !force && imp.revealed.get() == Some(cursor) {
            return;
        }
        imp.revealed.set(Some(cursor));
        let at = (!imp.reading.get()).then_some(cursor);
        let buffer = self.view().buffer();
        highlight::reveal_markers(&buffer, &imp.parsed.borrow(), at);
    }

    /// Whether the note is being read rather than edited.
    pub fn is_reading(&self) -> bool {
        self.imp().reading.get()
    }

    /// Switch between reading and editing.
    ///
    /// Reading mode is not just "hide the syntax": the view stops being
    /// editable and loses its caret. Typing into markup you cannot see is how
    /// you break a link without noticing, and a caret sitting in text that
    /// will not accept it is a lie about what the app is doing.
    pub fn set_reading(&self, reading: bool) {
        let imp = self.imp();
        if imp.reading.replace(reading) == reading {
            return;
        }
        self.apply_editability();
        self.close_completion();
        self.update_revealed(true);
    }

    fn apply_editability(&self) {
        let imp = self.imp();
        let editable = imp.has_note.get() && !imp.reading.get();
        let view = self.view();
        view.set_editable(editable);
        view.set_cursor_visible(editable);
    }

    fn cursor_line(&self) -> i32 {
        let buffer = self.view().buffer();
        buffer.iter_at_mark(&buffer.get_insert()).line()
    }

    /// One line's text, without its newline, and its character offset.
    fn line_text(&self, buffer: &gtk::TextBuffer, line: i32) -> (String, i32) {
        if line < 0 || line >= buffer.line_count() {
            return (String::new(), 0);
        }
        let Some(start) = buffer.iter_at_line(line) else {
            return (String::new(), 0);
        };
        let end = match buffer.iter_at_line(line + 1) {
            Some(mut next) => {
                // Step back over the newline, which is not part of the line.
                next.backward_char();
                next
            }
            None => buffer.end_iter(),
        };
        (buffer.text(&start, &end, true).to_string(), start.offset())
    }

    fn line_offset(&self, buffer: &gtk::TextBuffer, line: i32) -> i32 {
        match buffer.iter_at_line(line) {
            Some(iter) => iter.offset(),
            None => buffer.char_count(),
        }
    }

    fn view(&self) -> gtk::TextView {
        self.imp()
            .view
            .borrow()
            .clone()
            .expect("the view is built in constructed")
    }

    /// Put a note's body in the buffer without reporting it as an edit.
    pub fn load(&self, body: &str) {
        let view = self.view();
        self.imp().loading.set(true);
        let buffer = view.buffer();
        buffer.set_text(body);
        buffer.place_cursor(&buffer.start_iter());
        self.imp().loading.set(false);
        self.close_completion();
        // The change handler was suppressed, so styling is applied here — a
        // note must arrive already styled, not on the first keystroke.
        self.restyle_all();
    }

    /// Apply a formatting action to the selection, or at the cursor.
    ///
    /// One undoable step: the buffer's own undo stack groups everything
    /// between `begin_user_action` and `end_user_action`, so Ctrl+Z takes the
    /// whole formatting back rather than one bracket at a time.
    pub fn apply_format(&self, format: Format) {
        // The formatting buttons are insensitive while reading, but an
        // accelerator or a test can still get here.
        if self.imp().reading.get() {
            return;
        }
        let view = self.view();
        let buffer = view.buffer();
        buffer.begin_user_action();

        match format.edit() {
            Edit::Wrap { before, after } => self.wrap_selection(&buffer, format, &before, &after),
            Edit::Prefix { prefix } => self.prefix_lines(&buffer, &prefix),
            Edit::Block { text, caret } => self.insert_block(&buffer, &text, caret),
        }

        buffer.end_user_action();
        view.grab_focus();
    }

    /// Wrap the selection in `before`/`after`, or take an existing wrapping
    /// off.
    ///
    /// Pressing Bold twice used to give `****text****`. Whether the text is
    /// already emphasised is a question for the scanner — the same one that
    /// decides how it is styled — so this asks it rather than matching
    /// characters by hand.
    fn wrap_selection(
        &self,
        buffer: &gtk::TextBuffer,
        format: markdown::Format,
        before: &str,
        after: &str,
    ) {
        if self.unwrap_existing(buffer, format) {
            return;
        }

        let (mut start, mut end) = match buffer.selection_bounds() {
            Some(bounds) => bounds,
            None => {
                let at = buffer.iter_at_mark(&buffer.get_insert());
                (at, at)
            }
        };
        let selected = buffer.text(&start, &end, true).to_string();

        buffer.delete(&mut start, &mut end);
        let at = start.offset();
        let mut insertion = start;
        buffer.insert(&mut insertion, &format!("{before}{selected}{after}"));

        let inner = at + before.chars().count() as i32;
        if selected.is_empty() {
            // Nothing was selected, so the caret belongs between the markers,
            // ready to type.
            buffer.place_cursor(&buffer.iter_at_offset(inner.min(buffer.char_count())));
            return;
        }

        // The text stays selected, and selected *inside* the markers. Leaving
        // the caret after the run instead meant a second press could not tell
        // it was already emphasised, so pressing Bold twice nested a second
        // pair rather than taking the first off.
        let end = (inner + selected.chars().count() as i32).min(buffer.char_count());
        buffer.select_range(&buffer.iter_at_offset(inner), &buffer.iter_at_offset(end));
    }

    /// Remove the markers around the run the cursor or selection is inside,
    /// if it is inside one of this format's. Reports whether it did.
    fn unwrap_existing(&self, buffer: &gtk::TextBuffer, format: markdown::Format) -> bool {
        let Some(style) = format.style() else {
            return false;
        };

        let (from, to) = match buffer.selection_bounds() {
            Some((start, end)) => (start.offset() as usize, end.offset() as usize),
            None => {
                let at = buffer.iter_at_mark(&buffer.get_insert()).offset() as usize;
                (at, at)
            }
        };

        let text = self.body();
        let parsed = markdown::parse(&text);

        // The run has to contain the selection, not merely touch it, or
        // pressing Bold just after a bold word would unbold that word.
        let Some(span) = parsed
            .spans
            .iter()
            .find(|span| span.style == style && span.start <= from && span.end >= to)
        else {
            return self.remove_empty_pair(buffer, format);
        };

        let opening = parsed
            .markers
            .iter()
            .find(|marker| marker.end == span.start);
        let closing = parsed
            .markers
            .iter()
            .find(|marker| marker.start == span.end);
        let (Some(opening), Some(closing)) = (opening, closing) else {
            return false;
        };

        let (span_start, span_end) = (span.start as i32, span.end as i32);

        // How much of each marker belongs to *this* format. `***both***` is one
        // marker of three, and taking all of it off would remove the italic
        // along with the bold.
        let chars: Vec<char> = text.chars().collect();
        let (before, after) = match format.edit() {
            Edit::Wrap { before, after } => (before, after),
            // Every format with a style wraps; this is unreachable.
            _ => return false,
        };
        let opening_width = Self::owned_width(&chars[opening.start..opening.end], &before) as i32;
        let closing_width = Self::owned_width(&chars[closing.start..closing.end], &after) as i32;

        // From the inside out, so what is left is the delimiter of the style
        // that stays: `***both***` unbolds to `*both*`, not `**both**` with a
        // stray asterisk. And the closing one first, because deleting the
        // opening one would shift it.
        let deletions = [
            (closing.start as i32, closing.start as i32 + closing_width),
            (opening.end as i32 - opening_width, opening.end as i32),
        ];
        for (from, to) in deletions {
            let mut start = buffer.iter_at_offset(from);
            let mut end = buffer.iter_at_offset(to);
            buffer.delete(&mut start, &mut end);
        }

        // Leave the text that was emphasised selected, so pressing again puts
        // the emphasis back rather than starting an empty pair beside it.
        let start = span_start - opening_width;
        let end = span_end - opening_width;
        buffer.select_range(
            &buffer.iter_at_offset(start.max(0)),
            &buffer.iter_at_offset(end.clamp(0, buffer.char_count())),
        );
        true
    }

    /// How many characters of `marker` this format's delimiter owns.
    ///
    /// A run of one repeated character — `***`, `___`, `~~` — may be shared by
    /// two styles at once, so a format takes only as many as it writes and
    /// leaves the rest. Anything else comes off whole: a link's closing
    /// `](target)` is as long as the target, and taking the six characters of
    /// the literal `](url)` would leave the rest of it in the note.
    fn owned_width(marker: &[char], delimiter: &str) -> usize {
        let want = delimiter.chars().count();
        let repeated = marker
            .first()
            .is_some_and(|first| marker.iter().all(|c| c == first));
        if repeated && marker.len() > want && want > 0 {
            want
        } else {
            marker.len()
        }
    }

    /// The `**|**` case: a pair with nothing between it yet, which the scanner
    /// does not report as a span because there is no content to style.
    fn remove_empty_pair(&self, buffer: &gtk::TextBuffer, format: markdown::Format) -> bool {
        let markdown::Edit::Wrap { before, after } = format.edit() else {
            return false;
        };
        if buffer.selection_bounds().is_some() {
            return false;
        }

        let at = buffer.iter_at_mark(&buffer.get_insert()).offset();
        let text = self.body();
        let chars: Vec<char> = text.chars().collect();

        let opening: Vec<char> = before.chars().collect();
        let closing: Vec<char> = after.chars().collect();
        let start = at - opening.len() as i32;
        if start < 0 || at as usize + closing.len() > chars.len() {
            return false;
        }

        let precedes = chars[start as usize..at as usize] == opening[..];
        let follows = chars[at as usize..at as usize + closing.len()] == closing[..];
        if !(precedes && follows) {
            return false;
        }

        let mut from = buffer.iter_at_offset(start);
        let mut to = buffer.iter_at_offset(at + closing.len() as i32);
        buffer.delete(&mut from, &mut to);
        true
    }

    /// Put `prefix` on every selected line, or take it off if they all have it.
    fn prefix_lines(&self, buffer: &gtk::TextBuffer, prefix: &str) {
        let (start, end) = buffer.selection_bounds().unwrap_or_else(|| {
            let at = buffer.iter_at_mark(&buffer.get_insert());
            (at, at)
        });
        let (first, last) = (start.line(), end.line());

        let line_text = |line: i32| -> String {
            let Some(from) = buffer.iter_at_line(line) else {
                return String::new();
            };
            let to = match buffer.iter_at_line(line + 1) {
                Some(mut next) => {
                    next.backward_char();
                    next
                }
                None => buffer.end_iter(),
            };
            buffer.text(&from, &to, true).to_string()
        };

        // Toggling: a second press on an already-quoted block unquotes it,
        // which is what makes these buttons safe to press twice.
        let all_prefixed = (first..=last).all(|line| line_text(line).starts_with(prefix));

        for line in (first..=last).rev() {
            let Some(mut at) = buffer.iter_at_line(line) else {
                continue;
            };
            if all_prefixed {
                let mut to = buffer.iter_at_offset(at.offset() + prefix.chars().count() as i32);
                buffer.delete(&mut at, &mut to);
            } else {
                buffer.insert(&mut at, prefix);
            }
        }
    }

    /// Insert a block on lines of its own, below whatever the cursor is on.
    fn insert_block(&self, buffer: &gtk::TextBuffer, text: &str, caret: usize) {
        let mut at = buffer.iter_at_mark(&buffer.get_insert());
        let prefix = if at.starts_line() { "" } else { "\n" };
        let start = at.offset() + prefix.chars().count() as i32;

        buffer.insert(&mut at, &format!("{prefix}{text}"));
        let landing = (start + caret as i32).min(buffer.char_count());
        buffer.place_cursor(&buffer.iter_at_offset(landing));
    }

    /// The buffer, for tests that need to place a cursor or a selection.
    pub fn buffer_for_test(&self) -> gtk::TextBuffer {
        self.view().buffer()
    }

    /// How many embeds the view is drawing, counted off the widget tree rather
    /// than the bookkeeping — an overlay left behind by a deleted embed is
    /// still on screen however tidy the list of them looks.
    pub fn embeds_drawn_for_test(&self) -> usize {
        // The overlays are not direct children of the view: GTK parents each
        // to a private `GtkTextViewChild`. So this walks the subtree looking
        // for the slots by their class.
        fn count(widget: &gtk::Widget) -> usize {
            // `get_visible`, not `is_visible`: the latter also asks whether
            // every ancestor is visible, and a test window is never shown.
            let mut found =
                usize::from(widget.has_css_class(attachments::SLOT) && widget.get_visible());
            let mut child = widget.first_child();
            while let Some(next) = child {
                found += count(&next);
                child = next.next_sibling();
            }
            found
        }
        count(self.view().upcast_ref())
    }

    /// The scan the buffer is currently styled against, for tests.
    pub fn parsed(&self) -> Parsed {
        self.imp().parsed.borrow().clone()
    }

    /// How many full re-scans have happened, for tests.
    pub fn full_rescans_for_test(&self) -> u32 {
        self.imp().full_rescans.get()
    }

    /// Follow a link at a character offset, for tests.
    pub fn follow_link_at_for_test(&self, offset: usize) -> bool {
        self.follow_link_at(offset)
    }

    /// Accept a completion candidate, for tests.
    pub fn accept_completion_for_test(&self, chosen: &str) {
        self.accept_completion(chosen);
    }

    /// Put the caret at a character offset, for tests and previews.
    pub fn place_cursor_at(&self, offset: usize) {
        let buffer = self.view().buffer();
        buffer.place_cursor(&buffer.iter_at_offset(offset as i32));
    }

    /// The note as it reads on screen, with the hidden syntax left out.
    ///
    /// Asserting on this rather than on which tag sits where is the point:
    /// what matters is what a reader sees, not how it was achieved.
    pub fn visible_text_for_test(&self) -> String {
        let buffer = self.view().buffer();
        let Some(marker) = buffer.tag_table().lookup(highlight::MARKER) else {
            return self.body();
        };
        let mut visible = String::new();
        let mut iter = buffer.start_iter();
        while !iter.is_end() {
            if !iter.has_tag(&marker) {
                visible.push(iter.char());
            }
            iter.forward_char();
        }
        visible
    }

    /// Type `text` at a character offset, as a user would.
    ///
    /// Exists so `tests/widgets.rs` can drive the incremental re-scan through
    /// the same path a keystroke takes — placing the cursor first, because
    /// which line is re-scanned is decided by where the cursor is.
    pub fn insert_at_for_test(&self, offset: usize, text: &str) {
        let buffer = self.view().buffer();
        let mut iter = buffer.iter_at_offset(offset as i32);
        buffer.place_cursor(&iter);
        buffer.insert(&mut iter, text);
    }

    /// The text as it stands.
    pub fn body(&self) -> String {
        let buffer = self.view().buffer();
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), true)
            .to_string()
    }

    pub fn grab_focus_to_text(&self) {
        self.view().grab_focus();
    }

    /// Whether there is anything to edit. An editor with no note open is shown
    /// but not typeable, rather than swapped for a placeholder — swapping loses
    /// the scroll position and the focus ring.
    /// Whether a note is open. Reading mode can still take editing away.
    pub fn set_editable(&self, editable: bool) {
        self.imp().has_note.set(editable);
        self.apply_editability();
    }

    /// Words in the buffer, for the status line.
    pub fn word_count(&self) -> usize {
        self.body().split_whitespace().count()
    }
}

/// The character offset just past an unclosed `[[` before `cursor`, if the
/// cursor is inside one.
///
/// Scans backwards from the cursor and gives up at a line break or a `]]`,
/// so a `[[` three paragraphs above does not turn every subsequent keystroke
/// into a completion query.
fn open_link_start(chars: &[char], cursor: usize) -> Option<usize> {
    let mut index = cursor;
    while index > 0 {
        index -= 1;
        match chars[index] {
            '\n' => return None,
            ']' => return None,
            '[' if index > 0 && chars[index - 1] == '[' => return Some(index + 1),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::open_link_start;

    fn find(text: &str) -> Option<usize> {
        let chars: Vec<char> = text.chars().collect();
        open_link_start(&chars, chars.len())
    }

    #[test]
    fn an_open_bracket_pair_starts_a_query() {
        assert_eq!(find("See [["), Some(6));
        assert_eq!(find("See [[Borrow"), Some(6));
    }

    #[test]
    fn a_closed_link_is_not_a_query() {
        assert_eq!(find("See [[Borrow]]"), None);
        assert_eq!(find("See [[Borrow]] and more"), None);
    }

    #[test]
    fn a_bracket_on_an_earlier_line_does_not_leak() {
        // Otherwise every keystroke after an abandoned "[[" is a query.
        assert_eq!(find("See [[\nnext line"), None);
    }

    #[test]
    fn a_single_bracket_is_not_a_wikilink() {
        assert_eq!(find("See [label"), None);
        assert_eq!(find("plain text"), None);
    }

    #[test]
    fn queries_are_measured_in_characters() {
        let text = "🎉 [[Wör";
        let chars: Vec<char> = text.chars().collect();
        let start = open_link_start(&chars, chars.len()).expect("a query");
        let query: String = chars[start..].iter().collect();
        assert_eq!(query, "Wör");
    }
}
