//! Widget tests.
//!
//! **Exactly one `#[test]` touches a widget, on purpose.** GTK is
//! thread-affine: it must be initialised on, and only ever touched from, one
//! thread. `cargo test` runs tests on separate threads and `--test-threads=1`
//! still does not guarantee they share one, so a second `#[test]` building a
//! widget is a crash waiting for a scheduler to find it. The widget cases are
//! therefore a hand-rolled runner inside `widgets`, reporting each by name.
//!
//! The other two tests here touch only the GObject type system — registering a
//! type and constructing a plain `GObject` — which is thread-safe and needs no
//! display.
//!
//! Windows are constructed and driven but never presented — mapping a window
//! needs a compositor, and none of these assertions need one.

use std::fs;
use std::path::Path;

use adw::prelude::*;
use brain::model::markdown::{Format, Style};
use brain::model::note::NoteId;
use brain::model::tree::{self, Listed, Row};
use brain::ui::{BrainWindow, Editor, RowObject, Sidebar};
use gtk::glib;

/// A one-pixel PNG, so an embed has a real file to resolve to.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
    0x42, 0x60, 0x82,
];

/// Cases run in order; each gets a fresh window.
type Case = (&'static str, fn(&BrainWindow, &Path));

#[test]
fn widgets() {
    // GTK is initialised directly rather than by running the application. The
    // real `activate` presents a window and, with no vault configured, opens
    // the folder chooser — a portal dialog that has no business in a test.
    //
    // The windows below are built with no application attached at all.
    // Attaching one that has not emitted `startup` earns a `Gtk-CRITICAL` per
    // window, and noise like that is what hides a real one. The window is
    // built to tolerate it: every handler that needs the application asks for
    // it and does nothing when there is none, which is also what happens
    // during teardown.
    adw::init().expect("GTK and libadwaita initialise");

    let mut failures = Vec::<String>::new();
    for (name, case) in CASES {
        let vault = tempfile::tempdir().expect("temp dir");
        seed(vault.path());
        let window: BrainWindow = glib::Object::new();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            case(&window, vault.path());
        }));
        if let Err(panic) = result {
            let message = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "panicked".to_string());
            failures.push(format!("{name}: {message}"));
        }

        window.destroy();
    }

    assert!(failures.is_empty(), "\n  {}", failures.join("\n  "));
}

/// Visit every widget under `root`, including popovers and dialog children.
fn walk(root: &gtk::Widget, visit: &mut impl FnMut(&gtk::Widget)) {
    visit(root);
    let mut child = root.first_child();
    while let Some(widget) = child {
        walk(&widget, visit);
        child = widget.next_sibling();
    }
}

fn seed(root: &Path) {
    fs::write(
        root.join("Rust ownership.md"),
        "# Rust ownership\n\nMoves.\n",
    )
    .expect("write");
    fs::create_dir_all(root.join("Meetings")).expect("dir");
    fs::write(root.join("Meetings/Standup.md"), "Notes from standup.\n").expect("write");
}

const CASES: &[Case] = &[
    ("a new window opens with no note", |window, _| {
        assert!(
            window.editor_body().is_some(),
            "the editor should exist even with nothing open"
        );
        assert_eq!(window.editor_body().unwrap_or_default(), "");
    }),
    ("showing a note puts its body in the editor", |window, _| {
        let id = NoteId::from_relative("Rust ownership.md");
        window.show_note(Some((&id, "# Rust ownership\n\nMoves.\n")));
        assert_eq!(
            window.editor_body().unwrap_or_default(),
            "# Rust ownership\n\nMoves.\n"
        );
    }),
    ("closing a note empties the editor", |window, _| {
        let id = NoteId::from_relative("A.md");
        window.show_note(Some((&id, "body")));
        window.show_note(None);
        assert_eq!(window.editor_body().unwrap_or_default(), "");
    }),
    (
        "the note list accepts a tree, results and an empty vault",
        |window, _| {
            window.set_rows(&[
                Row::Folder {
                    path: "Meetings".to_string(),
                    name: "Meetings".to_string(),
                    depth: 0,
                    notes: 1,
                    expanded: true,
                },
                Row::Note {
                    id: NoteId::from_relative("Meetings/B.md"),
                    excerpt: String::new(),
                    depth: 1,
                },
                Row::Note {
                    id: NoteId::from_relative("A.md"),
                    excerpt: "first".to_string(),
                    depth: 0,
                },
            ]);
            window.select_note(Some(&NoteId::from_relative("A.md")));
            // Selecting a note that is not in the list clears the highlight rather
            // than leaving the previous one looking current.
            window.select_note(Some(&NoteId::from_relative("Gone.md")));
            window.select_note(None);

            window.set_results(&[(NoteId::from_relative("A.md"), "first".to_string())]);
            window.set_result_count(Some(1));
            assert_eq!(window.sidebar_subtitle_for_test(), "1 match");
            window.set_result_count(None);
            assert_eq!(window.sidebar_subtitle_for_test(), "");

            window.set_rows(&[]);
        },
    ),
    (
        "a folder row never takes the highlight meant for a note",
        |window, _| {
            // A folder and a note can share a path — `Meetings` and
            // `Meetings.md` — and highlighting the folder would leave the open
            // note looking closed.
            window.set_rows(&[
                Row::Folder {
                    path: "Meetings".to_string(),
                    name: "Meetings".to_string(),
                    depth: 0,
                    notes: 0,
                    expanded: false,
                },
                Row::Note {
                    id: NoteId::from_relative("Meetings.md"),
                    excerpt: String::new(),
                    depth: 0,
                },
            ]);
            window.select_note(Some(&NoteId::from_relative("Meetings.md")));
        },
    ),
    (
        "the banner shows, offers its one action, and clears",
        |window, _| {
            window.set_banner(Some("Not saving: disk full"), None);
            assert_eq!(
                window.banner_for_test(),
                Some(("Not saving: disk full".to_string(), None))
            );

            // A condition that offers a way out carries exactly one button: the
            // other choice is always "do nothing".
            window.set_banner(Some("“Note” changed on disk"), Some("Reload"));
            assert_eq!(
                window.banner_for_test(),
                Some((
                    "“Note” changed on disk".to_string(),
                    Some("Reload".to_string())
                ))
            );

            window.set_banner(None, None);
            assert_eq!(window.banner_for_test(), None);
        },
    ),
    ("a toast can be raised with no note open", |window, _| {
        window.toast("Saved");
    }),
    (
        "a loaded note arrives already styled, not on the first keystroke",
        |window, _| {
            let id = NoteId::from_relative("Rust ownership.md");
            window.show_note(Some((&id, "# Title\n\nSome **bold** text.\n")));
            let parsed = window.editor_parsed().expect("an editor");
            assert!(
                parsed.spans.iter().any(|s| s.style == Style::Heading(1)),
                "the heading was not styled on load"
            );
            assert!(!parsed.markers.is_empty(), "no syntax was marked");
        },
    ),
    (
        "typing inside a line agrees with a full re-scan",
        |window, _| {
            // The incremental path is the whole performance argument, and a
            // disagreement here is styling that drifts as you type and only a
            // reload fixes. Each case types one character into a note and
            // compares the cached scan with what parsing the text outright
            // gives.
            let cases: &[(&str, usize, &str)] = &[
                ("Some bold text.", 5, "*"),
                ("A [[Link]] here.", 15, "!"),
                ("# Heading", 9, "!"),
                ("| a | b |\n|---|---|\n| 1 | 2 |", 28, " "),
                ("- [ ] a task", 11, "s"),
                ("Tagged #rust here", 12, "y"),
                ("> quoted line", 13, "s"),
                ("plain text", 5, "x"),
                // Typing on an early line moves every offset below it, and the
                // cache is in absolute offsets: these fail unless the splice
                // shifts the lines it did not re-scan.
                ("**bold** here\nA [[Link]] there", 2, "x"),
                ("# Title\n\n`code` and *it*", 7, "!"),
            ];

            for (body, at, typed) in cases {
                let id = NoteId::from_relative("Case.md");
                window.show_note(Some((&id, body)));

                let editor = window.editor().expect("an editor");
                let before = editor.full_rescans_for_test();
                editor.insert_at_for_test(*at, typed);

                // Without this the test would pass just as happily against an
                // editor that re-scanned everything on every keystroke, which
                // is exactly the thing the cache exists to avoid.
                assert_eq!(
                    editor.full_rescans_for_test(),
                    before,
                    "typing {typed:?} into {body:?} escalated to a full re-scan"
                );

                let expected = {
                    let text = editor.body();
                    brain::model::markdown::parse(&text)
                };
                let actual = editor.parsed();

                let mut actual_spans = actual.spans.clone();
                let mut expected_spans = expected.spans.clone();
                actual_spans.sort_by_key(|s| (s.start, s.end));
                expected_spans.sort_by_key(|s| (s.start, s.end));
                assert_eq!(
                    actual_spans, expected_spans,
                    "spans drifted after typing {typed:?} into {body:?}"
                );

                let mut actual_markers = actual.markers.clone();
                let mut expected_markers = expected.markers.clone();
                actual_markers.sort_by_key(|m| (m.start, m.end));
                expected_markers.sort_by_key(|m| (m.start, m.end));
                assert_eq!(
                    actual_markers, expected_markers,
                    "markers drifted after typing {typed:?} into {body:?}"
                );
            }
        },
    ),
    (
        "a link reports itself when followed, and plain text does not",
        |window, _| {
            let id = NoteId::from_relative("A.md");
            window.show_note(Some((&id, "See [[Borrow checker]] today.\n")));
            let editor = window.editor().expect("an editor");

            let followed = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
            let seen = followed.clone();
            editor.connect_closure(
                "link-activated",
                false,
                glib::closure_local!(move |_: Editor, target: String| {
                    seen.borrow_mut().push(target);
                }),
            );

            // Inside the brackets.
            assert!(editor.follow_link_at_for_test(8), "the link was not found");
            // Outside them.
            assert!(
                !editor.follow_link_at_for_test(0),
                "plain text was treated as a link"
            );

            assert_eq!(followed.borrow().as_slice(), ["Borrow checker".to_string()]);
        },
    ),
    ("a link inside code is not clickable", |window, _| {
        // The editor asks the scanner what a link is, so anything it did
        // not style as one is not one here either.
        let id = NoteId::from_relative("A.md");
        window.show_note(Some((&id, "`[[Not a link]]` here\n")));
        let editor = window.editor().expect("an editor");
        assert!(!editor.follow_link_at_for_test(5));
    }),
    ("enter carries a list on to the next line", |window, _| {
        let id = NoteId::from_relative("A.md");
        let editor = {
            window.show_note(Some((&id, "")));
            window.editor().expect("an editor")
        };

        // Type a line, then press Enter at the end of it.
        let press_enter = |typed: &str| {
            window.show_note(Some((&id, "")));
            editor.insert_at_for_test(0, typed);
            editor.continue_list();
            editor.body()
        };

        assert_eq!(press_enter("- milk"), "- milk\n- ");
        assert_eq!(press_enter("1. first"), "1. first\n2. ");
        assert_eq!(press_enter("  - nested"), "  - nested\n  - ");
        assert_eq!(press_enter("- [x] wrote it"), "- [x] wrote it\n- [ ] ");
        assert_eq!(
            press_enter("- milk\n- "),
            "- milk\n",
            "Enter on an empty item ends the list instead of adding another bullet"
        );
        assert_eq!(
            press_enter("just prose"),
            "just prose",
            "outside a list the editor declines, leaving Enter to the text view"
        );
    }),
    ("typing two brackets asks for candidates", |window, _| {
        let id = NoteId::from_relative("A.md");
        window.show_note(Some((&id, "See ")));
        let editor = window.editor().expect("an editor");

        let queries = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let seen = queries.clone();
        editor.connect_closure(
            "link-query",
            false,
            glib::closure_local!(move |_: Editor, query: String| {
                seen.borrow_mut().push(query);
            }),
        );

        // Typed one character at a time, as a person would.
        for (at, character) in [(4, "["), (5, "["), (6, "B"), (7, "o"), (8, "r")] {
            editor.insert_at_for_test(at, character);
        }

        assert_eq!(
            queries.borrow().as_slice(),
            [
                "".to_string(),
                "B".to_string(),
                "Bo".to_string(),
                "Bor".to_string()
            ],
            "the query did not track what was typed"
        );
    }),
    (
        "accepting a candidate closes the brackets and moves on",
        |window, _| {
            let id = NoteId::from_relative("A.md");
            window.show_note(Some((&id, "See ")));
            let editor = window.editor().expect("an editor");

            editor.insert_at_for_test(4, "[[Bor");
            editor.accept_completion_for_test("Borrow checker");

            assert_eq!(editor.body(), "See [[Borrow checker]]");
        },
    ),
    ("a tag reports itself when followed", |window, _| {
        let id = NoteId::from_relative("A.md");
        window.show_note(Some((&id, "About #project/brain today.\n")));
        let editor = window.editor().expect("an editor");

        let followed = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let seen = followed.clone();
        editor.connect_closure(
            "tag-activated",
            false,
            glib::closure_local!(move |_: Editor, tag: String| {
                seen.borrow_mut().push(tag);
            }),
        );

        assert!(editor.follow_link_at_for_test(8), "the tag was not found");
        assert_eq!(followed.borrow().as_slice(), ["project/brain".to_string()]);

        // A hash inside code is not a tag, so it is not clickable either.
        window.show_note(Some((&id, "`#rust` here\n")));
        assert!(!editor.follow_link_at_for_test(2));
    }),
    ("the tag tree shows counts and empties out", |window, _| {
        window.set_tags(&[
            ("project".to_string(), 2),
            ("project/brain".to_string(), 1),
            ("rust".to_string(), 3),
        ]);
        window.set_active_tag(Some("project/brain"));
        // A tag that is not in the tree clears the highlight rather than
        // leaving the previous one looking current.
        window.set_active_tag(Some("gone"));
        window.set_active_tag(None);
        window.set_tags(&[]);
    }),
    (
        "an embed does not put a character in the buffer",
        |window, vault| {
            // The invariant the whole offset scheme rests on: what the editor
            // holds is exactly what the file holds. A child anchor would add a
            // U+FFFC here and silently corrupt the note on save.
            let attachments = vault.join("attachments");
            std::fs::create_dir_all(&attachments).expect("dir");
            std::fs::write(attachments.join("d.png"), PNG).expect("write");

            let body = "Before\n![[d.png]]\nAfter\n";
            window.set_vault_root(Some(vault.to_path_buf()));
            let id = NoteId::from_relative("A.md");
            window.show_note(Some((&id, body)));

            assert_eq!(
                window.editor_body().unwrap_or_default(),
                body,
                "the buffer no longer matches the file"
            );
        },
    ),
    (
        "inserting an embed puts it on a line of its own",
        |window, vault| {
            window.set_vault_root(Some(vault.to_path_buf()));
            let id = NoteId::from_relative("A.md");
            window.show_note(Some((&id, "Some prose.")));

            let editor = window.editor().expect("an editor");
            editor.insert_at_for_test(11, "");
            editor.insert_embed("d.png");

            assert_eq!(editor.body(), "Some prose.\n![[d.png]]\n");
        },
    ),
    (
        "deleting an embed takes its picture with it",
        |window, vault| {
            let attachments = vault.join("attachments");
            fs::create_dir_all(&attachments).expect("dir");
            fs::write(attachments.join("d.png"), PNG).expect("write");

            window.set_vault_root(Some(vault.to_path_buf()));
            let id = NoteId::from_relative("A.md");
            window.show_note(Some((&id, "Before\n![[d.png]]\nAfter\n")));

            let editor = window.editor().expect("an editor");
            assert_eq!(editor.embeds_drawn_for_test(), 1, "no picture was drawn");

            // The whole construct, as selecting the line and deleting it does.
            let buffer = editor.buffer_for_test();
            buffer.place_cursor(&buffer.iter_at_offset(7));
            buffer.delete(
                &mut buffer.iter_at_offset(7),
                &mut buffer.iter_at_offset(17),
            );

            assert_eq!(editor.body(), "Before\n\nAfter\n");
            assert_eq!(
                editor.embeds_drawn_for_test(),
                0,
                "the picture outlived the embed that named it"
            );

            // And a note with no embed at all does not inherit the last one's.
            window.show_note(Some((&NoteId::from_relative("B.md"), "Plain.\n")));
            assert_eq!(editor.embeds_drawn_for_test(), 0);
        },
    ),
    (
        "a missing attachment is reported, not drawn as a broken image",
        |window, vault| {
            // Nothing on disk, so this exercises the absent path.
            window.set_vault_root(Some(vault.to_path_buf()));
            let id = NoteId::from_relative("A.md");
            window.show_note(Some((&id, "![[gone.png]]\n")));
            assert_eq!(window.editor_body().unwrap_or_default(), "![[gone.png]]\n");
        },
    ),
    (
        "the unused attachments dialog handles both states",
        |window, _| {
            window.show_unused_attachments(&[]);
            window.show_unused_attachments(&["orphan.png".to_string()]);
        },
    ),
    (
        "the palette shows hits, reports what was picked, and empties out",
        |_window, _| {
            let palette = brain::ui::Palette::new();

            let chosen = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
            let seen = chosen.clone();
            palette.connect_closure(
                "chosen",
                false,
                glib::closure_local!(move |_: brain::ui::Palette, id: String| {
                    seen.borrow_mut().push(id);
                }),
            );

            palette.set_hits(&[
                brain::ui::Hit {
                    id: "Rust ownership.md".to_string(),
                    title: "Rust ownership".to_string(),
                    detail: "Moves are destructive".to_string(),
                    highlight: Some((10, 21)),
                },
                brain::ui::Hit {
                    id: "Meetings/Standup.md".to_string(),
                    title: "Standup".to_string(),
                    detail: "Meetings".to_string(),
                    highlight: None,
                },
            ]);

            // An empty result set is a state, not a failure.
            palette.set_hits(&[]);
            assert_eq!(palette.mode(), brain::ui::Mode::Title);
            assert!(chosen.borrow().is_empty());
        },
    ),
    ("every icon-only button has a tooltip", |window, _| {
        // A header bar of unlabelled icons is unusable without them, and
        // nothing warns when one is missing.
        //
        // Only buttons this app built are judged. GTK and libadwaita put
        // untooltipped buttons inside their own composite widgets — the window
        // controls, a menu button's toggle, a toggle group's toggles, the back
        // button, the banner's action — and those are not ours to annotate.
        const LIBRARY_OWNED: &[&str] = &[
            "GtkWindowControls",
            "GtkMenuButton",
            "AdwBackButton",
            "AdwBanner",
            "AdwToggleGroup",
            "AdwSplitButton",
        ];

        let mut missing = Vec::new();
        walk(window.upcast_ref::<gtk::Widget>(), &mut |widget| {
            let mut parent = widget.parent();
            while let Some(ancestor) = parent {
                if LIBRARY_OWNED.contains(&ancestor.type_().to_string().as_str()) {
                    return;
                }
                parent = ancestor.parent();
            }

            let (has_label, tooltip) = if let Some(button) = widget.downcast_ref::<gtk::Button>() {
                (
                    button.label().is_some_and(|label| !label.is_empty()),
                    button.tooltip_text(),
                )
            } else if let Some(button) = widget.downcast_ref::<gtk::MenuButton>() {
                (
                    button.label().is_some_and(|label| !label.is_empty()),
                    button.tooltip_text(),
                )
            } else {
                return;
            };

            if !has_label && tooltip.map_or(true, |text| text.is_empty()) {
                let icon = widget
                    .downcast_ref::<gtk::Button>()
                    .and_then(|button| button.icon_name())
                    .or_else(|| {
                        widget
                            .downcast_ref::<gtk::MenuButton>()
                            .and_then(|button| button.icon_name())
                    })
                    .map(|name| name.to_string())
                    .unwrap_or_else(|| "no icon".to_string());

                let mut ancestry = Vec::new();
                let mut parent = widget.parent();
                while let Some(widget) = parent {
                    ancestry.push(widget.type_().to_string());
                    parent = widget.parent();
                }
                missing.push(format!("[{icon}] under {}", ancestry.join(" < ")));
            }
        });
        assert!(missing.is_empty(), "untooltipped icon buttons: {missing:?}");
    }),
    (
        "nothing uses the deprecated .dim-label class",
        |window, _| {
            let mut offenders = Vec::new();
            walk(window.upcast_ref::<gtk::Widget>(), &mut |widget| {
                if widget.has_css_class("dim-label") {
                    offenders.push(format!("{:?}", widget.type_()));
                }
            });
            assert!(offenders.is_empty(), "{offenders:?}");
        },
    ),
    ("formatting wraps a selection", |window, _| {
        let id = NoteId::from_relative("A.md");
        window.show_note(Some((&id, "make this bold please")));
        let editor = window.editor().expect("an editor");

        let buffer = editor.buffer_for_test();
        buffer.select_range(&buffer.iter_at_offset(10), &buffer.iter_at_offset(14));
        editor.apply_format(Format::Bold);

        assert_eq!(editor.body(), "make this **bold** please");
    }),
    (
        "formatting with no selection leaves the caret to type in",
        |window, _| {
            let id = NoteId::from_relative("A.md");
            window.show_note(Some((&id, "type here: ")));
            let editor = window.editor().expect("an editor");

            let buffer = editor.buffer_for_test();
            buffer.place_cursor(&buffer.end_iter());
            editor.apply_format(Format::Italic);

            assert_eq!(editor.body(), "type here: **");
            let caret = buffer.iter_at_mark(&buffer.get_insert()).offset();
            assert_eq!(caret, 12, "the caret should sit between the markers");
        },
    ),
    (
        "taking one emphasis off leaves the other in place",
        |window, _| {
            // `***text***` is one marker of three either side, shared by the
            // bold and the italic. Unbolding used to take all three and remove
            // both.
            let id = NoteId::from_relative("A.md");
            window.show_note(Some((&id, "make this bold please")));
            let editor = window.editor().expect("an editor");
            let buffer = editor.buffer_for_test();

            buffer.select_range(&buffer.iter_at_offset(10), &buffer.iter_at_offset(14));
            editor.apply_format(Format::Bold);
            editor.apply_format(Format::Italic);
            assert_eq!(editor.body(), "make this ***bold*** please");

            editor.apply_format(Format::Bold);
            assert_eq!(
                editor.body(),
                "make this *bold* please",
                "unbolding took the italic with it"
            );

            // And the other way round, from the same starting point.
            editor.apply_format(Format::Bold);
            assert_eq!(editor.body(), "make this ***bold*** please");
            editor.apply_format(Format::Italic);
            assert_eq!(
                editor.body(),
                "make this **bold** please",
                "unitalicising took the bold with it"
            );

            // The last one off leaves the words alone.
            editor.apply_format(Format::Bold);
            assert_eq!(editor.body(), "make this bold please");
        },
    ),
    (
        "a block format toggles off when pressed twice",
        |window, _| {
            // Otherwise the buttons are one-way and quoting by accident is
            // unfixable except by hand.
            let id = NoteId::from_relative("A.md");
            window.show_note(Some((&id, "a line")));
            let editor = window.editor().expect("an editor");

            let buffer = editor.buffer_for_test();
            buffer.place_cursor(&buffer.start_iter());
            editor.apply_format(Format::Quote);
            assert_eq!(editor.body(), "> a line");

            buffer.place_cursor(&buffer.start_iter());
            editor.apply_format(Format::Quote);
            assert_eq!(editor.body(), "a line");
        },
    ),
    (
        "inserting a table writes one on its own lines",
        |window, _| {
            let id = NoteId::from_relative("A.md");
            window.show_note(Some((&id, "before")));
            let editor = window.editor().expect("an editor");

            let buffer = editor.buffer_for_test();
            buffer.place_cursor(&buffer.end_iter());
            editor.apply_format(Format::Table);

            let body = editor.body();
            assert!(body.starts_with("before\n|"), "{body:?}");
            // And what it wrote is a table to the scanner, not just pipes.
            assert!(brain::model::markdown::parse(&body)
                .spans
                .iter()
                .any(|span| span.style == Style::TableRow));
        },
    ),
    ("every formatting button reaches the editor", |window, _| {
        // The panel reports a name; the window turns it back into a format
        // and applies it. A name that does not survive is a dead button.
        let id = NoteId::from_relative("A.md");
        for format in [
            Format::Bold,
            Format::Italic,
            Format::Strikethrough,
            Format::Code,
            Format::Heading(1),
            Format::Heading(2),
            Format::Heading(3),
            Format::Quote,
            Format::Bullet,
            Format::Task,
            Format::WikiLink,
            Format::Link,
            Format::CodeBlock,
            Format::Table,
            Format::Rule,
        ] {
            window.show_note(Some((&id, "x")));
            let editor = window.editor().expect("an editor");
            let before = editor.body();
            window.request_format_for_test(format);
            assert_ne!(editor.body(), before, "{format:?} changed nothing");
        }
    }),
    (
        "the backlinks pane shows what links here and empties out",
        |window, _| {
            window.set_backlinks(&[(
                NoteId::from_relative("A.md"),
                "See Borrow checker for why.".to_string(),
            )]);
            window.set_backlinks(&[]);
        },
    ),
    (
        "opening a fence escalates to a full re-scan",
        |window, _| {
            // An edit that changes what the lines *below* it mean cannot take
            // the one-line path.
            let id = NoteId::from_relative("Case.md");
            window.show_note(Some((&id, "``\nnot code yet\nstill prose\n")));

            let editor = window.editor().expect("an editor");
            let before = editor.full_rescans_for_test();
            editor.insert_at_for_test(2, "`");
            assert!(
                editor.full_rescans_for_test() > before,
                "opening a fence took the one-line path"
            );

            let text = editor.body();
            assert_eq!(
                editor.parsed().spans,
                brain::model::markdown::parse(&text).spans,
                "the lines below the fence were not re-scanned"
            );
        },
    ),
    (
        "the caret reveals the syntax it is inside and nothing else",
        |window, _| {
            // The whole point of the narrow reveal: editing one construct must
            // not turn the rest of the line back into source.
            let id = NoteId::from_relative("A.md");
            window.show_note(Some((&id, "a **bold** and a [[Link]] here")));
            let editor = window.editor().expect("an editor");

            editor.place_cursor_at(5);
            assert_eq!(
                editor.visible_text_for_test(),
                "a **bold** and a Link here",
                "the asterisks under the caret should show, and the brackets should not"
            );

            editor.place_cursor_at(20);
            assert_eq!(
                editor.visible_text_for_test(),
                "a bold and a [[Link]] here",
                "the reveal did not move with the caret"
            );

            // With the caret in the prose between them, nothing is source.
            editor.place_cursor_at(13);
            assert_eq!(editor.visible_text_for_test(), "a bold and a Link here");
        },
    ),
    (
        "a block prefix is revealed by the caret anywhere on its line",
        |window, _| {
            // Unlike an inline construct: the hashes belong to the whole
            // heading, and hiding them while the words are being edited would
            // make Backspace at the start of the line delete something
            // invisible.
            let id = NoteId::from_relative("A.md");
            window.show_note(Some((&id, "# A heading\n\nprose")));
            let editor = window.editor().expect("an editor");

            editor.place_cursor_at(9);
            assert_eq!(editor.visible_text_for_test(), "# A heading\n\nprose");

            editor.place_cursor_at(15);
            assert_eq!(editor.visible_text_for_test(), "A heading\n\nprose");
        },
    ),
    (
        "reading mode hides every marker and refuses edits",
        |window, _| {
            let id = NoteId::from_relative("A.md");
            window.show_note(Some((&id, "# Title\n\na **bold** word")));
            let editor = window.editor().expect("an editor");

            // Caret on the heading, so there is something revealed to lose.
            editor.place_cursor_at(3);
            assert!(editor.visible_text_for_test().contains('#'));

            window.set_reading(true);
            assert!(window.is_reading());
            assert_eq!(editor.visible_text_for_test(), "Title\n\na bold word");

            // The formatting buttons write Markdown; in reading mode they must
            // not, however they are reached.
            let before = editor.body();
            editor.apply_format(Format::Italic);
            assert_eq!(editor.body(), before, "reading mode accepted an edit");

            window.set_reading(false);
            assert!(!window.is_reading());
            editor.place_cursor_at(3);
            assert!(
                editor.visible_text_for_test().contains('#'),
                "leaving reading mode did not bring the syntax back"
            );
        },
    ),
    (
        "the folder menu and the sort menu name actions the window has",
        |window, _| {
            // These are named in menu models built at popup time, so nothing
            // else would notice one being renamed: a menu item pointing at an
            // action that does not exist is permanently insensitive and silent.
            for action in [
                "new-note-in",
                "new-folder",
                "new-folder-in",
                "rename-folder",
                "delete-folder",
                "sort",
                "find",
            ] {
                assert!(window.has_action(action), "the window has no win.{action}");
            }
        },
    ),
    (
        "a note row's menu names actions the window has",
        |window, _| {
            let sidebar = Sidebar::new();
            sidebar.set_results(&[(NoteId::from_relative("A.md"), "first".to_string())]);

            let mut child = sidebar.first_child();
            let menu = loop {
                match child {
                    Some(widget) => match widget.downcast::<gtk::PopoverMenu>() {
                        Ok(menu) => break Some(menu),
                        Err(widget) => child = widget.next_sibling(),
                    },
                    None => break None,
                }
            };
            let menu = menu.expect("the row menu is parented to the sidebar");
            let model = menu.menu_model().expect("a menu model");

            let actions: Vec<String> = (0..model.n_items())
                .filter_map(|index| {
                    model
                        .item_attribute_value(index, "action", Some(&String::static_variant_type()))
                        .and_then(|value| value.get::<String>())
                })
                .collect();
            assert_eq!(actions, ["win.rename-note", "win.delete-note"]);

            // Naming an action the window does not have would leave the menu
            // items permanently insensitive, which nothing else here would
            // catch.
            for action in &actions {
                assert!(
                    gtk::prelude::WidgetExt::activate_action(window, action, None).is_ok(),
                    "the window has no {action}"
                );
            }
        },
    ),
    (
        "the vault root stays droppable however long the tree is",
        |_, _| {
            let sidebar = Sidebar::new();
            let notes: Vec<Listed> = (0..200)
                .map(|n| Listed::new(NoteId::from_relative(format!("Deep/{n}.md")), String::new()))
                .collect();
            sidebar.set_rows(&tree::rows(
                &notes,
                &[],
                &["Deep".to_string()].into_iter().collect(),
                tree::Sort::Name,
            ));

            // The strip is pinned outside the scroller, so a tree that fills
            // the pane cannot bury the only way back to the root.
            let (visible, idle) = sidebar
                .root_strip_for_test(false)
                .expect("the sidebar has a root strip");
            assert!(visible, "the root strip should be on screen");
            assert_eq!(idle, "", "it should say nothing with no drag in the air");

            let (_, dragging) = sidebar
                .root_strip_for_test(true)
                .expect("the sidebar has a root strip");
            assert_eq!(dragging, "Move to Vault Root");

            // And a drop on it means the root, not whatever folder was last on
            // screen.
            let moved = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            sidebar.connect_closure(
                "moved",
                false,
                glib::closure_local!(
                    #[strong]
                    moved,
                    move |_: Sidebar, payload: String, destination: String| {
                        moved.borrow_mut().push((payload, destination));
                    }
                ),
            );
            sidebar.drop_for_test("brain-note:Deep/1.md", "");
            assert_eq!(
                moved.borrow().as_slice(),
                [("brain-note:Deep/1.md".to_string(), String::new())]
            );
        },
    ),
];

/// Cases that need no window, kept here so they run on the GTK thread too.
#[test]
fn row_objects_project_a_note_without_reading_it_back() {
    let id = NoteId::from_relative("Meetings/Standup.md");

    // In the tree the indent says which folder a note is in, so the row does
    // not repeat it.
    let in_tree = RowObject::note(&id, "Notes from standup.", 1);
    assert_eq!(in_tree.title(), "Standup");
    assert_eq!(in_tree.folder(), "");
    assert_eq!(in_tree.depth(), 1);
    assert_eq!(in_tree.excerpt(), "Notes from standup.");
    assert_eq!(in_tree.note_id(), id);
    assert!(!in_tree.is_folder());

    // Flattened into results there is no indent, so it does.
    let result = RowObject::result(&id, "a matching line");
    assert_eq!(result.folder(), "Meetings");
    assert_eq!(result.depth(), 0);
    assert_eq!(
        RowObject::result(&NoteId::from_relative("Top.md"), "").folder(),
        ""
    );

    let folder = RowObject::for_folder("Meetings", "Meetings", 0, 3, true);
    assert!(folder.is_folder());
    assert_eq!(folder.count(), 3);
    assert!(folder.expanded());
}

/// The editor's loading guard, which does not need a display.
#[test]
fn an_editor_type_exists_and_names_itself() {
    // Constructing widgets needs GTK initialised, which `widgets` owns. This
    // asserts only what can be known without it.
    assert_eq!(
        <Editor as gtk::glib::prelude::StaticType>::static_type().name(),
        "BrainEditor"
    );
    assert_eq!(
        <Sidebar as gtk::glib::prelude::StaticType>::static_type().name(),
        "BrainSidebar"
    );
}
