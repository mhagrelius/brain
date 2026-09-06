# Brain

A Markdown notebook for Omarchy, in C++ with Qt 6 / Qt Quick.

Your notes are ordinary `.md` files in a folder you choose. Delete Brain and
they are untouched; put the folder in git and you have history; open it in any
other editor and it reads the same. Nothing Brain writes into a note is
unreadable to `cat`.

This is the Omarchy port. The GNOME app (Rust, GTK 4) is on the `main` branch,
along with `brain-server`, the container that holds the shared vault and its
vectors on the NAS. The two clients speak the same wire format and read the
same vault.

## Features

- **Plain files.** A vault is a folder of `.md` files with optional YAML
  frontmatter. Only `tags`, `aliases`, `created` and `updated` are understood;
  everything else is preserved byte for byte.
- **One editing view.** Source is styled as you type — headings, emphasis,
  code, quotes, lists, tasks, rules, tables — with the syntax hidden outside
  the construct the caret is in. `Ctrl+E` hides it everywhere and stops edits.
- **Lists that continue themselves.** Enter repeats the indent and bullet, the
  next number, or a fresh unticked checkbox; Enter on an empty item ends the
  list; Backspace takes the bullet off.
- **Wikilinks.** `[[` completes over titles and aliases, `Ctrl+Click` or
  `Ctrl+Return` follows, a dead link offers to write the note, and renaming
  repoints every link that pointed at it. The rail lists what links here.
- **Tags.** `#tag` inline and `tags:` in frontmatter, nested as
  `#project/brain`, in a tree that filters the vault.
- **Folders.** The sidebar is the vault's own directory tree: drag to move
  notes and folders, right-click for the rest.
- **Search.** The header field filters the tree as you type. `Ctrl+K` goes to
  a note by title; `Ctrl+Shift+F` searches every note's text — BM25 over the
  words fused with vectors from an embedding server when one is reachable, so
  "why is my bread so flat" finds the note about hydration. No server is a
  supported state: search is the words alone.
- **Sync.** Every machine keeps a full local replica; the server stores and
  refuses stale writes. A conflict is a note beside the original, never a
  dialog.
- **Capture.** `Ctrl+Shift+N` (or `brain capture` from a Hyprland bind) drops
  a line into `Inbox.md` without leaving what you were doing.
- **Attachments.** Drop a file or paste an image; it is copied into
  `attachments/` and embedded, and pictures draw under the line.
- **It writes carefully.** Saves coalesce on a two-second tick and go out
  through a temporary file and a rename; edits made outside Brain are noticed
  and offered back.

## Build and install

```sh
bin/build                                   # → build/brain
cd build && ctest --output-on-failure       # the gate
cmake --install build --prefix ~/.local     # binary, .desktop, icon
```

Or `makepkg -si` in `packaging/`. Depends on `qt6-base`, `qt6-declarative`,
`qt6-svg`, `xdg-desktop-portal`.

## Pointing it at the NAS

Brain assumes `brain-server` at `http://mattnas:8082` and an embedding model at
`http://mattnas:8081`. Sync and the shared vector store stay off until a token
is set, because half a configuration is treated as none:

```sh
brain config sync_token    <the token from the container's .env>
brain config vectors_token <the same token>
brain status               # probes /health and says what is configured
```

The same keys live in `~/.config/brain/config.json` — the file the GNOME app
uses too — as `sync_url`, `sync_token`, `vectors_url`, `vectors_token` and
`embedding_url`. An empty `embedding_url` turns vectors off.

## Keys

`1`–`3` views · `Ctrl+N` new note · `Ctrl+K` go to note · `Ctrl+Shift+F`
search text · `Ctrl+F` or `/` filter the tree · `Ctrl+E` reading mode ·
`Ctrl+/` or a right-click in the editor for the insert menu (`Ctrl+Space` too, where fcitx5 does not own it) · `Ctrl+Shift+N` capture · `Ctrl+S` save now ·
`Ctrl+Shift+S` sync and search status · `Shift+F10` row menu.

## How it works

```
src/core/      the vault, notes, index, search, vectors, sync — QtCore only, ctest-covered
src/net/       the sockets: a blocking HTTP client, the embedder, the server client
src/backend.*  the App singleton: timers, watcher, worker threads, every row QML shows
src/editor.*   the editing model: source ↔ display map, hidden syntax, formats
src/qml/       Main, the sidebar, the note pane, the rail, the summoned surfaces
docs/handoff/  the design this was built from
```

The vault is canonical and the index derived. `Notebook` is the only thing
that writes a file. The scanner reports which characters are syntax, not just
what is styled — that is the whole editing model, ported from `quill`.

### Built differently from the GNOME app

- Frontmatter is not shown in the editor; the rail edits tags and aliases.
- The formatting panel is an insert menu (`Ctrl+Space`).
- Re-styling re-scans the whole note rather than one line: the scanner is
  microseconds on a note and the per-line cache was there for GTK's tag
  churn, which a QTextDocument does not have.
- Undo is Brain's own, over the source, since the document's undo stack
  would otherwise remember the hiding and revealing of syntax.

## Licence

GPL-3.0-or-later.
