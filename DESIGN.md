# Brain — design for review

A GTK 4 / libadwaita Markdown notebook for GNOME, in Rust. Local-first, single
user, no account. Built the way Stickies and Planner are built: a GTK-free
`model/` half that `cargo test` exercises with no display, an imperative `ui/`
half of `glib::wrapper!` subclasses, no blueprint, no `.ui` XML, no meson, no
async runtime.

## Scope

**The vault is the product.** Notes are ordinary `.md` files in a folder you
choose. Brain is a good editor and index over that folder and nothing more — if
Brain is deleted the notes are untouched, and if you `git init` the vault you
get history for free. Nothing Brain writes into a note is unreadable to `cat`,
Obsidian, or a text editor.

No sync, no publishing, no plugin system, no mobile. Those are all somebody
else's problem, and the file-per-note format means somebody else can solve them.

## What it does

### The vault

```
~/Notes/                      chosen once, remembered in ~/.config/brain/config.json
  Rust ownership.md
  Meetings/
    2026-07-30 standup.md
  attachments/
    diagram.png
  .brain/                     Brain's own directory, disposable
```

One note is one file. The filename stem is the title; renaming the note renames
the file. Folders are organisational only — they are not namespaces and nothing
resolves by them unless a link says so.

There is no index cache on disk. See "Built differently" — it was measured
before it was built, and the numbers did not justify it.

### The note

Frontmatter is optional and, when present, restricted:

```markdown
---
tags: [rust, learning]
aliases: [Ownership]
created: 2026-07-30
updated: 2026-07-31
---

# Rust ownership

Moves are **destructive**. See [[Borrow checker]] and #rust.
```

Only those four keys are understood. **Unknown keys and unknown formatting are
preserved verbatim on round-trip** — Brain rewrites the lines it owns and copies
the rest through byte for byte, so a note that came from somewhere else does not
get mangled. This is why there is no YAML dependency: the supported subset is
line-oriented `key: value` and `[a, b]` lists, which is a hundred lines of
exhaustively testable parsing, and a real YAML parser would tempt the format to
grow.

### Editing

One `GtkTextView` and no preview pane. You are always looking at the source and
it is always styled: headings scale up and bold, `**bold**` renders bold, code
gets a mono face and a background, block quotes get a rule, `[[links]]` go blue
and follow on click, `#tags` get a chip background.

**Syntax characters are hidden except in the construct the caret is inside.**
The mechanic is Stickies': one `md-marker` `TextTag` carrying `invisible`,
applied to every marker span. Stickies toggles that one tag for the whole note
on focus, which is right for a sticky and wrong for a window whose editor holds
focus all day. So the tag is invisible throughout and removed from the markers
of whatever the caret is in — the scanner gives each marker the extent of its
own construct, and both halves of a pair share it, so `**` never comes back
without its partner. A block prefix — the `##`, the `>`, a bullet's indent —
takes its whole line as that extent instead, since it belongs to the line and
Backspace at the line start must not delete something invisible.

Editing one construct therefore leaves the rest of the line rendered, which the
earlier per-line reveal did not: with the caret anywhere on a line, every link
and every emphasis on it turned back into source.

**Reading mode**, `Ctrl+E`, reveals nothing at all and makes the view
non-editable with no caret. Still the same widget — switching does not reflow
or rescroll the note, which is the whole reason there is no second view. It
also takes the formatting buttons out of service and is remembered across
launches, and it is why a plain click follows a link there: with no cursor to
place, the modifier has nothing to disambiguate.

Image embeds are the one exception to "text only": a `GtkPicture` is drawn on
the line beneath `![[diagram.png]]`, which is itself hidden like any other
syntax — the picture is what the line says, and the filename comes back when
the caret is in it. Non-image attachments get a clickable chip instead, opening
in the default app.

### Links and backlinks

`[[` opens a completion popover over note titles and aliases. Accepting one that
doesn't exist yet creates the file. `Ctrl+Click` or `Enter` on a link follows it.
A collapsible right-hand pane lists every note linking here, each with the
surrounding sentence, so a backlink is readable without opening it.

Resolution is by title, case-insensitively, then by alias. Two notes with the
same title in different folders are an ambiguity Brain reports rather than
guesses at; `[[Meetings/2026-07-30 standup]]` disambiguates by path.
`[[Note|display text]]` is supported. Renaming a note rewrites every inbound
link in the vault, as one undoable action.

### Tags

`#tag` inline and `tags:` in frontmatter are the same thing and both feed the
sidebar. Nested tags (`#project/brain`) form a tree. Clicking a tag filters the
note list to it. Tags are never stored anywhere but in the notes themselves.

### Folders

The sidebar is the vault's directory tree. A folder row exists because a
directory does — including an empty one, since the folder you just made is the
folder you go looking for — and a note sits under it because its path says so.
Nothing about the tree is stored: expansion is view state in the config, and
order comes from a sort (name, last written, made), never from a manual
arrangement. Manual ordering would need a file of positions that is a second
source of truth about the vault, drifting the moment anything touches the notes
from outside, and folders that reshuffle by date are folders you cannot learn —
so folders stay alphabetical whatever the notes are sorted by.

Because order is a sort, a drag can only ever mean "move this somewhere else" —
there is no position between two rows to aim at. Dropping on a folder means into
it, and coming back out means the vault root, which is a strip pinned below the
list rather than the blank space under the last row: a tree taller than the pane
leaves no blank space, and the root has to stay reachable at every vault size.
The strip holds its height whether or not it is showing an outline, since one
that appeared mid-drag would shove the rows the drag was aimed at.

Drag a note onto a folder and it moves. That is one `fs::rename` and nothing
else: links resolve by title, the title has not changed, so no other note is
touched. Dragging a folder moves the whole subtree in one rename, and the index
is rebuilt rather than patched, because every id beneath it changes at once.
A folder is deleted only when empty — the file manager is a better place to
mean "and everything in it".

### Search

Search is in the sidebar, always visible, because past a few hundred notes
filtering the list *is* how the list is read. Typing filters to titles first
and then to text, each result carrying the line that matched. `Ctrl+F` focuses
it, Enter opens the top result, Escape gives the tree back.

The palette stays for going somewhere without leaving the keyboard. `Ctrl+K` is
quick open: fuzzy over titles, aliases and paths, ranked, arrow keys and Enter.
`Ctrl+Shift+F` is full text: substring and word matching over the in-memory
text index, results grouped by note with a snippet per hit and the match
highlighted.

### Attachments

Drop a file on the editor and it is copied into `attachments/`, deduplicated by
content hash, and an embed inserted at the cursor. Pasting an image from the
clipboard does the same with a timestamped filename. A vault-wide "unreferenced
attachments" list is offered on demand, never automatically deleted.

### Also in v1

New/rename/delete note, folders in the sidebar, per-note word count, `Ctrl+S` as
a no-op that flushes (autosave is real, but people press it anyway), undo/redo
within a note, dark mode following libadwaita, an external-change watcher so
edits made by git or another editor appear immediately.

### Deliberately not in v1

Graph view, daily notes and templates, canvas/whiteboard, PDF or HTML export,
outline/table-of-contents pane, split panes and tabs, LaTeX and Mermaid, sync,
plugins, i18n. Each is additive over this foundation, and the graph view in
particular is a lot of custom drawing for something people screenshot once.

## Architecture

```
src/
  model/                     no GTK — cargo test with no display
    note.rs                  the record: path, title, frontmatter, body
    frontmatter.rs           the restricted parser, verbatim round-trip
    markdown/
      scan.rs                source → styled spans, block-incremental
      links.rs               [[wikilink]] and #tag extraction
    vault.rs                 the folder: scan, read, atomic write, rename
    index.rs                 titles, aliases, links, backlinks, tags, text
    search.rs                fuzzy title match and full-text query
    tree.rs                  folders and notes flattened into sidebar rows
    config.rs                the one thing outside the vault: which vault
  ui/
    application.rs           owns the vault and index; the only mutator
    window.rs                split views, breakpoint, view stack
    sidebar.rs               folder tree / search results, drag and drop
    editor.rs                the TextView, tags, keybindings, autosave tick
    highlight.rs             spans → TextTags applied to the buffer
    link_popover.rs          [[ completion
    backlinks_panel.rs       the 360px right-hand pane
    palette.rs               Ctrl+K and Ctrl+Shift+F
    attachments.rs           drop target, clipboard paste, embed widgets
    style.css, style-dark.css
```

**The vault is canonical, the index is derived.** Widgets emit signals of
intent; `BrainApplication` is the single place that writes a file or mutates the
index. A `dirty` set plus a `glib::timeout_add_local` tick coalesces saves so
typing never blocks on I/O; writes go tmp → fsync → rename, per note; the buffer
is also flushed on note switch, window close and `shutdown`.

**Re-styling is per-block, not per-keystroke-per-document.** A keystroke
re-scans the block containing the cursor and re-applies tags to that range only.
The scanner is line-based, so a block is a line plus the fence/frontmatter state
it starts in; that state is cached per line, and only an edit that changes it —
a fence opened, a frontmatter delimiter touched — escalates to a full re-scan. This is what keeps a 5,000-word note responsive, it is what
makes revealing markers in the cursor's block cheap, and it is the one
performance decision in the whole design that has to be right up front.

**External changes come from `gio::FileMonitor`,** one per directory in the
vault, not the `notify` crate. `gio` is already a dependency and its monitors
deliver on the GLib main loop, so there is no channel and no thread. A change to
the open note with unsaved local edits raises a banner offering reload or
keep — it never silently overwrites either side.

**Widget tree in Rust, styled by `include_str!`'d CSS,** with Adwaita `var(--…)`
tokens wherever the theme has a meaning. Outer `Adw.OverlaySplitView` for the
sidebar, inner one packed `END` at 360sp for backlinks, one `Adw.Breakpoint` at
`675sp` collapsing both. `Adw.Toast` for undo of destructive actions.

**Nothing async.** GLib timers for the save tick and the search debounce;
`glib::spawn_future_local` if anything ever needs to await. The initial vault
scan is synchronous behind a spinner — a thousand notes is a few megabytes of
`read_to_string`.

## Testing

The same four layers. Unit tests inline per model module, and the ratio should
be lopsided towards `frontmatter.rs`, `markdown/`, `index.rs` and `search.rs` —
they are pure functions over strings and that is exactly where the bugs are.
`tests/session.rs` drives a real vault in a `tempfile::TempDir` through whole
scenarios: create, link, rename and watch the inbound links rewrite, delete,
rescan and assert the index matches a cold scan. One `tests/widgets.rs` with a
hand-rolled case runner, because GTK is thread-affine. `./test.sh` runs fmt,
clippy `-D warnings`, and tests, with `--headless` under Xvfb.

Two dedicated harnesses: a corpus of `.md` files with expected span output, so
styling regressions are a diff; and a round-trip property — read any note, write
it back unchanged, assert the bytes are identical.

`examples/preview.rs` renders the editor, sidebar and backlinks pane to PNG.

## Dependencies

`gtk4`, `libadwaita`, `gio`, `serde`, `serde_json`, `chrono`. Exactly Planner's
set, nothing new.

**No `pulldown-cmark`**, and this was reconsidered rather than assumed. A
general Markdown library reports *what* is styled but not *which characters are
syntax*, and marker hiding is the entire editing model — deriving markers from
it means treating the gaps between an inline span and its text children as
syntax, which is inference rather than fact, and every span additionally needs a
byte→char offset conversion before `GtkTextBuffer` will take it. That is two
subtle layers over a parser whose output shape is wrong for this, to replace
Stickies' `model/markdown.rs`, which already emits `Span` + `Marker` in char
offsets and is tested. Brain ports it and extends it: wikilinks, embeds, tags,
task checkboxes, thematic breaks, frontmatter as its own block, bare URLs.

The inherited limit is that inline styling does not cross a line break, because
the scanner is line-based. That is also what makes the incremental re-scan
possible, and hard-wrapped emphasis is rare in a wrapping editor.

Tables are the one place a line needs to know about the line *after* it: a row
of pipes is a header only if a delimiter row follows, and without that lookahead
every sentence containing a `|` becomes a table. So the scanner takes the next
line as an argument. Rows require a leading pipe — GFM does not, but "yes | no"
is commoner in notes than a borderless table, and misreading a sentence as a
table is the worse failure. The pipes stay visible for the same reason list
bullets do: a text view cannot draw column rules, so the pipes *are* the table.

Icons, `.desktop`, metainfo, and cargo+bash packaging scripts follow Stickies.

## Milestones

1. ~~Model core: vault scan, note, frontmatter round-trip, span scanner, link
   and tag extraction, index, search. No UI. All tested.~~
2. ~~Shell: window, sidebar tree, plain editor, open/save/autosave, new, rename,
   delete, first-run vault picker.~~
3. ~~Live styling: headings, emphasis, code, fences, quotes, lists, rules,
   tables, and marker hiding outside the caret's construct.~~
4. ~~Wikilinks: `[[` completion, follow, create-on-follow, backlinks pane,
   rename rewrites inbound links.~~
5. ~~Tags: sidebar tag tree, filter.~~
6. ~~Attachments: drop, paste, inline images, chips, unreferenced list.~~
7. ~~Search: `Ctrl+K` palette, `Ctrl+Shift+F` full text with snippets.~~
8. ~~External change watching, packaging.~~ (The cache was measured and
   dropped.)
9. ~~The folder tree: drag to move notes and folders, folder create/rename/
   delete, a sort, and search in the sidebar rather than only in a dialog.~~

## Built differently, or not built

Where the finished thing differs from this document, this is what happened.

- **The editor holds the whole file, not the body.** The design styles
  frontmatter as a recessed block in the editor, which is only possible if the
  editor can see it. Round-tripping costs nothing, since an untouched block is
  written back byte for byte.
- **Block quotes are indented and italic, not ruled.** `GtkTextTag` styles
  runs of text and cannot draw a border; a rule needs custom drawing on the
  text view, which is a job of its own for a line down the left of a quote.
- **Headings 4, 5 and 6 share one treatment.** Below the third level the
  difference between sizes is smaller than the noise.
- **A ticked task is not struck through.** The strikethrough landed on the
  `[x]` rather than the task, which read as the box being cancelled.
- **A link is followed with Ctrl+Click or Ctrl+Return, not Enter.** Plain
  Enter is how you start a new line; binding it to navigation would make the
  editor unusable inside a link. The pointer turns to a hand over a link, so
  the modifier is discoverable rather than folklore.
- **Renaming is not undoable.** It rewrites every inbound link, which is the
  right behaviour, but reversing it means restoring several files at once and
  there is no undo stack spanning notes yet. Renaming back does the same work
  in reverse, which is not the same promise.
- **`***both***` is the one nesting the scanner handles.** Nested emphasis was
  out of scope until the formatting buttons made it reachable in one click:
  italicising something already bold writes exactly that, and without it the
  run parsed as bold wrapped round a stray asterisk. Both styles are reported
  over the same text, and each comes off independently — which is why an
  unwrap removes as many characters as the format writes, taken from the
  inside of the marker outwards, rather than the whole marker: one marker of
  three cannot say whether two characters or one are being asked for, and
  taking all three unbolded and unitalicised in one press. Markers that are
  not a run of one repeated character still come off whole, since a link's
  closing `](target)` is as long as its target.
- **The right pane carries Details as well as Backlinks.** Asked for after
  the first install: something like LibreOffice's sidebar, saying what the note
  is and what can be done to it. The formatting half is the interesting part —
  each button shows the syntax it writes, so the pane teaches the Markdown
  rather than hiding it behind a toolbar. `Format` lives in `model/markdown`
  beside the scanner, and a test parses what every button writes to prove the
  scanner styles it: a button that emitted syntax the editor rendered as plain
  prose would be worse than no button.
- **The reveal is per-construct, and there is a reading mode.** The design said
  no modes and syntax revealed in the cursor's block, on Stickies' precedent.
  Stickies could hide everything on focus-out because focus genuinely leaves a
  sticky; here the editor holds focus for the whole session, so "the cursor's
  block" meant one full line of raw source on screen at all times, flickering as
  the caret moved. Two changes: each marker now carries the extent of its own
  construct and is revealed only from inside that, so editing `**bold**` leaves
  the link beside it rendered; and `Ctrl+E` switches to reading, where nothing
  is revealed and the view takes no edits. It is still one widget — the mode
  changes what the marker tag covers and whether the view is editable, not
  which widget is on screen, so nothing reflows or rescrolls.
- **The editor is clamped to a readable measure.** Prose set to the full width
  of a maximised window loses the eye between lines; `AdwClamp` holds it near
  80 characters and centres it, which is what GNOME apps showing a document do.
- **The window remembers its size**, which the design never mentioned and the
  HIG expects. A maximised window records that it was maximised rather than its
  maximised size, so unmaximising returns to the shape it had before.
- **There is a keyboard shortcuts dialog** (`AdwShortcutsDialog`) and an About
  entry. Every shortcut in the app was otherwise folklore. It is written by
  hand rather than derived from the accel map, because the map knows the keys
  but not which are worth telling someone about or what to call them.
- **Editor colours are derived from the view's own foreground colour**, tinted,
  rather than from a light/dark branch containing black and white. `GtkTextTag`
  takes colours rather than CSS classes, so they cannot come from the
  stylesheet; deriving them from the resolved foreground keeps them correct
  under any theme, including a high-contrast one.
- **Lists nest, with a hanging indent**, ported from Stickies after it was
  built there. Depth comes from the indent widths a note actually uses rather
  than a fixed number of spaces per level, so two-space and four-space notes
  both nest one step at a time and a note mixing them still nests
  monotonically. The spaces before a bullet become syntax, since the level's
  margin does that job and leaving them in would indent twice over. The nesting
  stack rides through the editor's per-line cache alongside `LineState`, which
  is why it is a fixed-size `Copy` stack rather than a `Vec`.
- **Paragraph styles are extended back to the line start when applied.**
  `GtkTextView` reads left margin, indent, spacing and paragraph background
  from a line's *first* character, so a heading tag beginning after its `## `,
  or a list tag beginning after its bullet, silently does nothing. This cost
  two rounds of "the margin is not applied" before it was found — once for
  embeds reserving space, once for list indents.
- **Embeds are drawn as text-view overlays, not child anchors.** A
  `GtkTextChildAnchor` puts a real `U+FFFC` in the buffer, and every offset in
  the app assumes the buffer holds exactly the file's text — an anchor would
  shift the scanner's spans and corrupt notes on save.
  `gtk_text_view_add_overlay` positions a widget at a buffer coordinate and
  touches no text. The cost is that an overlay does not reflow text, so the
  room for it is reserved separately by a `pixels-below-lines` tag, made per
  height and applied once the picture has been measured.
- **An overlay cannot be removed, so the overlays are pooled.**
  `gtk_text_view_remove` knows anchored children and the gutter windows; an
  overlay is parented to a private `GtkTextViewChild` and matches neither, so
  it warns and does nothing, and `unparent` leaves it in GTK's internal list to
  be allocated and drawn for ever. Each embed is therefore drawn in a slot that
  is added once and refilled on every re-scan, with the spare slots hidden.
  Until this was found, every keystroke stacked another copy of each picture on
  the last, and deleting an embed left its image on screen.
- **The incremental re-scan shifts the lines it did not re-scan.** The cache is
  in absolute offsets and an edit moves every offset below it, so the splice
  works from the character count as well as the line count: spans and markers
  below the edited window are dropped by their *old* bounds and re-added moved
  by the difference.
- **`.brain/index.json` was not built.** The design justified it as making a
  cold start on a large vault fast, so it was measured first: a 1000-note vault
  reads off disk in 1.8 ms and indexes in 104 ms. The reading was never the
  problem, and the indexing turned out to be parsing every note four times over
  — once each for its links, its tags, its stripped text and its excerpt.
  Parsing once brought it to 29 ms, which is a third of a frame for a vault
  larger than most people have. A cache would now save 29 ms at the cost of a
  second copy of the vault that can disagree with the first, which is the
  expensive kind of complexity. If a vault ever gets big enough to want one,
  the measurement is the thing to repeat, not this decision.
- **Quick open and full-text search are one dialog with two modes.** They
  differ in what they match and in nothing else, and switching keeps the query
  — `Ctrl+Shift+F` after a fruitless `Ctrl+K` searches the text for what you
  already typed.
- **Attachments are deduplicated by name and contents, not by content hash.**
  Re-dropping the same file reuses its attachment; a different file of the same
  name gets a numeric suffix. Hashing every attachment to catch the same image
  saved under two names is work for a case nobody has yet complained about.
- **Ctrl+Click on a `#tag` in the editor filters by it**, which the design
  did not ask for. It costs one branch beside the link handler and is the
  obvious thing to try once tags are styled as chips.
- **The sidebar is a flat list of rows, not a tree widget.** `TreeListModel`
  would put the hierarchy inside GTK, where the expansion rules are neither
  testable without a display nor readable beside the vault they describe.
  Instead `model/tree.rs` takes the notes, the folders on disk and the set of
  open folders and returns rows carrying their own depth; the widget draws an
  indent and a chevron and knows nothing about nesting. The whole of "what does
  the sidebar look like" is then a pure function with a table of cases.
- **A drag carries a prefixed string, not a custom content type.** `brain-note:`
  and `brain-folder:` on one `String` payload: a drop target registered for
  both types would have to ask which it received anyway, and a payload from
  another application is then something to ignore rather than something to
  misread as a path.
- **The sidebar stats the vault only when a time sort is on.** Sorting by name
  is the default and the list is rebuilt on every save, so reading `mtime` for
  every note each time would be work for a field nobody asked about. Creation
  time falls back to modification time, because not every filesystem keeps a
  birth time and a note sorted to the bottom for ever on ext3 is worse than one
  sorted approximately.
- **Search filters the sidebar; the palette is still there.** They answer
  different questions — "narrow this list to what I mean" against "take me
  there without touching the mouse" — and the sidebar entry is the one that
  scales with the vault, so it is the one that is always on screen.
- **`examples/preview.rs` grows the window until something is drawn.**
  `WidgetPaintable` declines to draw a scroller whose content overflows it, so
  a fixed height per picture had to be re-guessed every time the seed note grew.

## Settled

- App id `us.hagreli.Brain`, binary `brain`, GObject classes `Brain*`.
- Notes are files. There is no database, no import step, and no "library".
- Titles come from filenames. A `# Heading` on line one is just a heading.
- The editor shows source. There is no read mode to toggle into.
- Frontmatter keys outside the known four are preserved, never interpreted.
- **Flatpak reaches the vault through the file portal.** `GtkFileDialog` returns
  a path the sandbox can use and the document store keeps that grant across
  restarts, so the manifest keeps Stickies' and Planner's posture of no
  `--filesystem` at all. `--filesystem=home` is the fallback only if a grant
  held for the app's lifetime turns out not to survive in practice, and taking
  it would be a documented retreat, not a shrug. Deb and `install.sh` are
  unaffected either way.
