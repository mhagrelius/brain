# Brain

A Markdown notebook for GNOME, in Rust with GTK 4 and libadwaita.

Your notes are ordinary `.md` files in a folder you choose. Delete Brain and
they are untouched; put the folder in git and you have history; open it in any
other editor and it reads the same. Nothing Brain writes into a note is
unreadable to `cat`.

## Install

```sh
./install.sh          # builds and installs into ~/.local — no root
./uninstall.sh        # removes it, and never touches a vault
```

Or build a package:

```sh
packaging/build-deb.sh
packaging/build-flatpak.sh
```

### Requirements

GTK 4.16 or newer (the stylesheet uses CSS custom properties), libadwaita 1.9,
and a Rust toolchain of 1.80 or newer.

## Using it

On first launch Brain asks for a folder to keep notes in. Point it at an
existing folder of Markdown files if you have one.

**The editor shows source, always styled.** Headings scale up, `**bold**`
renders bold, tables line up in a monospace grid. The syntax characters are
hidden except in the construct the caret is inside — put it in `**bold**` and
its asterisks come back, while the link later on the same line stays rendered.
So a note reads as prose while staying plain text on disk.

**Reading mode.** `Ctrl+E`, or the eye in the header, hides the syntax
everywhere and stops the note accepting edits. There is no second widget and no
preview pane: it is the same view with the caret taken away, so the scroll
position never moves. In reading mode a plain click follows a link, since
there is no cursor to place.

**Links.** `[[` opens a completion over note titles and aliases. `Ctrl+Click`
or `Ctrl+Return` follows a link; following one that points nowhere offers to
write it. Renaming a note repoints every link that pointed at it. The right
pane lists what links here.

**Tags.** `#tag` inline and `tags:` in frontmatter are the same thing. They
nest — `#project/brain` sits under `project` — and clicking one filters the
list.

**Attachments.** Drop a file on the editor or paste an image; it is copied into
`attachments/` and embedded. The picture is drawn at its own shape in place of
the `![[…]]` that names it, and the filename reappears when the caret is in it.

**Search.** `Ctrl+K` goes to a note by title. `Ctrl+Shift+F` searches the text
of every note, with the match shown in context.

**Details.** `F10` opens a pane with the note's properties and a set of
formatting buttons, each showing the syntax it writes. They grey out while you
are reading.

Frontmatter is optional. Only `tags`, `aliases`, `created` and `updated` are
understood, and **everything else is preserved verbatim** — a note written by
another tool does not get mangled.

## How it works

```
src/
  model/                   no GTK — cargo test with no display
    markdown/              source → styled spans + hideable syntax markers
    frontmatter.rs         the restricted parser, verbatim round-trip
    note.rs                the record: id, frontmatter, body
    vault.rs               the folder: scan, atomic writes, attachments
    index.rs               titles, aliases, links, backlinks, tags, text
    search.rs              fuzzy titles and full-text query
    config.rs              the one thing outside the vault: which vault
  ui/
    application.rs         owns the vault and index; the only thing that writes
    window.rs              split views, breakpoint, dialogs
    editor.rs              the TextView, incremental re-styling, formatting
    highlight.rs           spans → TextTags
    sidebar.rs, tag_tree.rs, details_panel.rs, backlinks_panel.rs
    palette.rs             Ctrl+K and Ctrl+Shift+F
    attachments.rs         drop, paste, embedded images
    watcher.rs             gio::FileMonitor per directory
```

**The vault is canonical, the index is derived.** Widgets emit signals of
intent; `BrainApplication` is the single place that writes a file. Saves are
coalesced by a two-second tick, written tmp → fsync → rename, and flushed on
note switch, close and shutdown.

**The scanner reports which characters are syntax**, not just what is styled —
that is the whole editing model, and it is why there is no Markdown crate here.
See DESIGN.md.

**Re-styling is per line.** A keystroke re-scans the line under the cursor and
the one above it; only an edit that changes what follows escalates to a full
re-scan.

**Nothing async.** GLib timers for the save tick and the search debounce.

## Development

```sh
cargo build
./test.sh                       # fmt, clippy -D warnings, tests
./test.sh --headless            # the same under Xvfb
cargo run --example preview -- /tmp/preview [dark]
cargo run --example icons_check # every icon name resolves in the theme
```

### Tests

| Where | Covers |
|---|---|
| `src/model/**` | the scanner, frontmatter round-trip, index, search — the bulk |
| `tests/session.rs` | whole scenarios against a real vault, no GTK |
| `tests/widgets.rs` | widgets, in one `#[test]` because GTK is thread-affine |
| `tests/lifecycle.rs` | the real application, driven end to end |
| `tests/first_run.rs` | an empty vault and the first note made in it |

`examples/preview.rs` renders every pane to PNG in light and dark, so "does
this look right?" is answerable without a screenshot prompt.

## Licence

GPL-3.0-or-later.
