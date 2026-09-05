# Handoff: Brain — vault window (Quattro-era redesign)

## Overview

Brain is a Markdown notebook whose vault is an ordinary folder of `.md` files. It exists today as a
GTK 4 / libadwaita app for GNOME (`mhagrelius/brain`, Rust). This handoff covers a **redesign of the
app's UI** in the visual language the user already uses for their other Qt/Quickshell apps (Ledger,
Buddy): neutral dark greys, a numbered left nav with a slate-teal active fill, monospace for every
number/label/status and a sans face for names, one amber action per screen, and mono status rails.

The redesign changes **the skin and the chrome, not the editing model**. Brain's defining behaviour
is unchanged: a note is always shown as source and always styled, with the syntax characters hidden
everywhere except inside the construct holding the caret.

Two decisions made during design that differ from today's app and must be carried into the build:

1. **Frontmatter is not rendered in the editor.** It stays in the file byte-for-byte, but the editor
   opens on the note's title. `tags`, `aliases`, `created` and `updated` are read in the right rail.
   *Consequence to resolve in implementation:* the rail must become the editing surface for those
   keys, or they become read-only inside Brain.
2. **The formatting panel is gone.** The 15-button formatting grid from today's details pane is now a
   summoned, filter-as-you-type **insert menu** (`Ctrl+Space`). Every format still shows the literal
   syntax it writes, so the format is still taught rather than hidden.

Out of scope, explicitly dropped by the user during design: any desktop-shell integration (omarchy
bar widget, plugin manifest / `shell.json` surfaces, Setup › Plugins page) and a lock-screen view.
Build this as a plain application window.

## About the Design Files

The files in this bundle are **design references created in HTML** — prototypes showing intended
look and behaviour. They are **not production code to copy**. The task is to recreate these designs
in the target codebase's environment using its established patterns:

- If Brain stays Rust + GTK 4 / libadwaita, recreate this as widgets and a stylesheet in
  `src/ui/`, following the existing conventions there (widget trees built in Rust, no `.ui` XML, no
  Blueprint, no GResource; colours that `GtkTextTag` needs are derived in code, everything else in
  `style.css`).
- If Brain is being ported to Qt/QML (the user's other apps are Qt 6 / Wayland), recreate it as QML
  components with a token singleton.

Either way, the HTML is the specification of appearance and layout, not the implementation.

`.dc.html` files open directly in a browser; `support.js` must sit beside them.

## Fidelity

**High-fidelity.** Colours, type sizes, row heights, gutters and copy are final and are listed
exactly below. Recreate pixel-perfectly using the codebase's own widget/component library.

The one deliberate looseness: the note body text is Lorem-free real sample content about sourdough.
Treat the *content* as sample data and the *styling of each Markdown construct* as the spec.

---

## Design Tokens

### Colour

| Token | Hex | Used for |
|---|---|---|
| `window` | `#1c1c1c` | window background, sidebar, popover bodies |
| `panel` | `#212121` | cards in the right rail, inputs, buttons (secondary) |
| `row-selected` | `#252525` | selected tree row, selected menu row, popover header strip |
| `chip` | `#262626` | avatar squares, inline-code background |
| `chip-strong` | `#2c2c2c` | avatar square on a selected row |
| `border` | `#333333` | window border, card border, popover border |
| `border-control` | `#3a3a3a` | input and secondary-button borders, keycap borders |
| `hairline` | `#2a2a2a` | pane dividers, table rows inside cards, section separators |
| `track` | `#2e2e2e` | progress-bar track |
| `border-danger` | `#4a2f37` | destructive button border |
| `border-tag` | `#2f4348` | tag chip border |
| `nav-active` | `#384a48` | active nav row fill |
| `nav-active-fg` | `#cfe3df` | number + count inside the active nav row |
| `fg` | `#e9e9e9` | primary text |
| `fg-secondary` | `#c8c8c8` | inactive nav labels, unselected row labels, body detail |
| `fg-mono` | `#a0a0a0` | mono body copy |
| `fg-mono-label` | `#8a8a8a` | mono key labels in the rail, mono syntax column |
| `fg-dim` | `#6b6b6b` | mono counts, timestamps, uppercase section labels |
| `fg-dimmest` | `#5c5c5c` | keyhints, footnotes |
| `amber` | `#f0cf7a` | primary button fill, caret |
| `amber-fg` | `#e6bd63` | links, wikilinks, `/` prefix, plugin-id emphasis |
| `amber-badge` | `#e8a54a` | unread/attention counts, unticked `[ ]` |
| `green` | `#9ee065` | success: `clean`, `synced ●`, ticked `[x]`, `21°C` inline code |
| `cyan` | `#7ed0dc` | tags, JSON keys, progress fill |
| `pink` | `#ff6f8d` | destructive / conflict |

Primary-button text on amber is `#1c1c1c`.

### Typography

Two faces, and the split is a rule: **sans for names and prose, mono for every number, key, path,
timestamp, status and piece of Markdown syntax that stays visible.**

- Sans: `'Helvetica Neue', Helvetica, Arial, sans-serif`
- Mono: `'JetBrains Mono', ui-monospace, monospace` — weights 400 / 500 / 700

| Role | Face | Size | Weight | Other |
|---|---|---|---|---|
| App title ("Brain") | sans | 16px | 700 | `letter-spacing: -.01em` |
| Note H1 | sans | 24px | 700 | `line-height: 1.3` |
| Note H2 | sans | 18px | 700 | |
| Header title / card title | sans | 15 / 14px | 700 | |
| Row label, card heading | sans | 13.5px | 400 / 700 | 700 for folders + active nav |
| Menu row, button label | sans | 13px | 400 | |
| Small button | sans | 12.5px | 400 | 700 on amber |
| Note body | sans | 14.5px | 400 | `line-height: 1.8`, measure `74ch` |
| Inline code, table, revealed markers | mono | 12.5px | 400 | |
| Rail values, syntax column | mono | 12px | 400 | |
| Counts, timestamps, meta, notification body | mono | 11 / 11.5px | 400 | |
| Uppercase section label | mono | 10px | 400 | `letter-spacing: .16em`, uppercase |
| Keyhints, footnotes | mono | 10px | 400 | |
| Clock (lock/OSD-style numerals) | mono | 28px | 700 | |

### Geometry

| Token | Value |
|---|---|
| Radius, rows / inputs / buttons | 6px |
| Radius, cards / popovers | 8px |
| Radius, chips / keycaps | 4px |
| Radius, progress bars | 2px |
| Row height, nav + tree | 32px |
| Row height, menus | 30px |
| Control height | 28–32px |
| Sidebar width | 290px |
| Right rail width | 360px |
| Status rail height | 28px (26px in the sidebar) |
| Progress bar height | 4px (3px inline) |
| Popover shadow | `0 18px 40px rgba(0,0,0,.5)` |
| Menu shadow | `0 22px 50px rgba(0,0,0,.55)` |

### Spacing

- Sidebar gutter: `16px` (app title, section labels)
- Row container: `padding: 0 8px`; each row `padding: 0 12px`
- **The sidebar row grid (important, this was iterated):** every row in both the nav and the tree is
  `32px` tall, `display:flex; align-items:center; gap:14px`, with a **fixed 11px leading slot** that
  holds either the nav number or the tree's disclosure mark (`v` / `>`), then the label (`flex:1`),
  then a mono count right-aligned. Child notes get `padding-left: 37px` (12 + 11 + 14) so their
  titles land under folder names and under nav labels.
- Card padding: `14px`; card gap in the rail: `14px`
- Editor padding: `26px 28px 0`
- Section labels sit `20px` above / `6–8px` below their list

---

## Screens / Views

### 1. Vault window (`2a` in the design file)

**Purpose:** read and write notes. The whole app.

**Layout:** `1440 × 900` reference size, `1px solid #333` border, no client-side decorations
(tiled). Three columns: sidebar `290px` (fixed) · content (flex) · right rail `360px` (fixed).
Vertical dividers are `1px #2a2a2a`.

#### 1.1 Sidebar

Top to bottom:

1. **Vault header** — `padding: 16px 16px 14px`. "Brain" (sans 16/700). Below it, mono 11px `#6b6b6b`:
   `~/notes · 218 notes`.
2. **Numbered nav**, three rows on the grid described above:
   - `1 Notes` — active: fill `#384a48`, label 700 `#e9e9e9`, number and count `#cfe3df`, count `218`
   - `2 Tags` — count `24` in `#6b6b6b`
   - `3 Inbox` — count `3` in `#e8a54a` (attention colour, because unfiled notes are a to-do)
   - Numbers are the keyboard shortcuts (`1`–`3`); the sidebar footer states `1-3`.
   - **Attachments was deliberately removed from the nav** — attachments are visible inside the note
     they are embedded in, and the sweep for orphans stays a menu action.
3. **Separator** — `1px #2a2a2a` full width, `18px` above it, then a label row `padding: 14px 16px 8px`
   with `VAULT` (mono 10, `.16em`, uppercase, `#6b6b6b`) on the left and the sort control `name ↓`
   (mono 10, `#5c5c5c`) on the right. *This separation was specifically asked for: the nav is views,
   the vault is a file tree, and they must not read as one seven-item list.*
4. **Folder tree**, same 32px row grid. Folders: mark in the leading slot, name sans 13.5/700, note
   count mono 11. Notes: `padding-left: 37px`, name sans 13.5, relative mtime mono 11 (`2h`, `1d`,
   `3d`, `1w`, `2w`). Selected note row: fill `#252525`, label `#e9e9e9`. Root-level notes appear
   after the folders with `margin-top: 6px`.
5. **Footer rail** — `26px`, `border-top: 1px #2a2a2a`, mono 10 `#5c5c5c`:
   `Ctrl+K · insert Ctrl+Space` left, `1-3` right.

Nothing else. A "written this week" metric card was designed here and **cut as not useful** — do not
reintroduce a productivity metric in that corner.

#### 1.2 Content header

`padding: 14px 20px`, `border-bottom: 1px #2a2a2a`.

- Left: note title (sans 15/700) with a mono 11 `#6b6b6b` subtitle: `baking · updated 2026-09-02 · 412 words`
- Right: search field — `250px × 32px`, `#212121`, `1px #3a3a3a`, radius 6, `⌕` glyph mono 11
  `#5c5c5c`, placeholder "Search notes" sans 13 `#8a8a8a`, and a `/` keycap (mono 10, `1px #3a3a3a`,
  radius 4, `padding: 1px 5px`)
- Then the one primary action: `+ New note` — `32px` tall, `padding: 0 14px`, radius 6, fill `#f0cf7a`,
  text `#1c1c1c` sans 13/700

#### 1.3 Editor

`padding: 26px 28px 0`, body `max-width: 74ch`, sans 14.5 / `line-height 1.8`, colour `#e9e9e9`.
Markdown construct styling — this is the heart of the screen:

| Construct | Treatment |
|---|---|
| H1 | sans 24/700, `line-height 1.3`, first element in the note |
| H2 | sans 18/700, `padding-top: 22px` |
| Body | sans 14.5/1.8; paragraphs `padding-top: 8–10px` |
| Wikilink | `#e6bd63` with `border-bottom: 1px rgba(230,189,99,.4)` |
| Tag | mono 12.5, `#7ed0dc` |
| Revealed markers (`**` around the caret's construct) | mono 12.5, `#5c5c5c`; the wrapped text is sans 700 |
| Caret | `2px` wide, `#f0cf7a`, height `1.05em` |
| Inline code | mono 12.5, `#9ee065`, background `#262626`, `1px #323232`, radius 4, `padding: 1px 5px` |
| List item | `padding-left: 26px; text-indent: -15px`; the literal `-` bullet is mono `#6b6b6b` |
| Task, done | `[x]` mono 12.5 `#9ee065`; the text dims to `#6b6b6b` (dimmed, never struck through) |
| Task, open | `[ ]` mono 12.5 `#e8a54a` |
| Blockquote | `border-left: 2px #384a48`, `padding-left: 26px`, italic `#8a8a8a` |
| Table | mono 12.5, `1px #2a2a2a`, radius 6, background `#1f1f1f`, `padding: 8px 12px`; the `\|---\|` delimiter row is `#5c5c5c`; pipes stay visible |
| Embed | filename line mono 11 `#e6bd63`; the image below at `1px #323232`, radius 8 |
| Frontmatter | **not rendered** (see Overview) |

#### 1.4 Right rail

`360px`, `border-left: 1px #2a2a2a`, `padding: 16px`, cards stacked with `14px` gap. Cards:
`1px #323232`, radius 8, `#212121`, `padding: 14px`.

**Card 1 — "THIS NOTE"** (mono 10 uppercase label). A key/value table: rows `26px` tall separated by
`1px #2a2a2a` (no border on the last), key mono 11 `#8a8a8a` left, value mono 12 right:
`folder baking` · `words 412` · `created 2026-08-14` · `updated 2026-09-02` · `aliases starter notes` ·
`on disk clean` (value `#9ee065`). Below the table, `12px` down, tag chips: mono 11 `#7ed0dc`,
`1px #2f4348`, radius 4, `padding: 1px 6px`, `6px` gap — `#baking`, `#project/brain`.

**Card 2 — "Backlinks"** — sans 13.5/700 heading with a mono 11 `#6b6b6b` count (`2 notes`) opposite.
Each entry: a `26px` avatar square (radius 6, `#262626`, `1px #323232`, mono 11 `#a0a0a0` initial),
then the note title sans 13 and the linking line mono 11 `#6b6b6b` `line-height 1.5`. `10px` between
entries.

#### 1.5 Status rail

`28px`, `border-top: 1px #2a2a2a`, `padding: 0 16px`, mono 11 `#6b6b6b`.
Left: `editing · 412 words · 2 backlinks · vectors 218/218`.
Right: `synced 12:41` followed by a `#9ee065` `●`.

### 2. Quick capture (summoned panel, `Super+N`)

`560px` wide popover: `1px #333`, radius 8, `#1c1c1c`, popover shadow.

- Header `padding: 12px 14px`, `border-bottom: 1px #2a2a2a`: "Capture" sans 14/700 left,
  `→ Inbox.md` mono 11 `#6b6b6b` right (the destination is always visible)
- Input: `1px #3a3a3a`, radius 6, `#212121`, `padding: 8px 10px`, min-height 32, sans 13.5/1.7, with a
  mono `+` in `#e6bd63` as prefix. Typed text shows a live tag (`#baking`, mono 12 `#7ed0dc`) and an
  in-progress wikilink (`[[Starter Log`, `#e6bd63`) with the caret after it.
- **`[[` completion list** appears directly under the input (`1px #2a2a2a`, radius 6, `#1f1f1f`):
  30px rows, title sans 13 left, mono 10 `#6b6b6b` qualifier right (folder, or `alias: …`). Selected
  row `#252525`. Same candidate list the editor uses.
- Footer rail `28px`, `border-top: 1px #2a2a2a`, mono 10 `#5c5c5c`, `gap: 16px`:
  `return save` · `ctrl+return save + open` · `tab destination` · `esc discard`

### 3. Insert menu (summoned, `Ctrl+Space`) — replaces the formatting grid

`520px` popover, same shell as capture.

- Header holds only the filter field: `30px`, `1px #3a3a3a`, radius 6, `#212121`, mono `/` prefix in
  `#e6bd63`, typed query sans 13 with the amber caret, and `wraps the selection` mono 10 `#6b6b6b`
  right-aligned as the hint.
- Rows `30px`, `padding: 0 14px`, three columns: **label** (sans 13, fixed `130px`, matched
  characters bold `#e9e9e9` against `#c8c8c8`), **the syntax it writes** (mono 12 `#8a8a8a`, flex),
  **keybind** (mono 10 `#6b6b6b`, if any). Selected row `#252525`.
- A `1px #2a2a2a` divider then a `RECENT` label (mono 10 uppercase) and recently used formats.
- All fifteen formats from the scanner must be listed, label and syntax verbatim:
  `Bold **text**` · `Italic *text*` · `Strikethrough ~~text~~` · `Code \`code\`` · `Heading 1 # ` ·
  `Heading 2 ## ` · `Heading 3 ### ` · `Quote > ` · `List - ` · `Task - [ ] ` ·
  `Link to Note [[Note]]` · `Web Link [text](…)` · `Code Block \`\`\`` · `Table | a | b |` ·
  `Separator ---`
- Disabled in reading mode, exactly as the old formatting panel was.

### 4. Note search (unified search, `Ctrl+K` / `Ctrl+Shift+F`)

`660px` popover.

- Breadcrumb row: mono 10 uppercase `.14em` — `omarchy › brain › notes` with the last crumb in
  `#e6bd63`; right-aligned `bm25 + vectors, fused`. (If shell context is dropped entirely, keep the
  right-hand note about the ranking and drop the breadcrumb.)
- Query field: `32px`, `1px #3a3a3a`, radius 6, `#212121`, mono `/` prefix `#e6bd63`, query sans 13.5,
  amber caret.
- Result rows `padding: 9px 14px`: `26px` avatar square, then title (sans 13.5, 700 when selected)
  over a mono 11 `#6b6b6b` context line of the form `folder · matched line` with the matched span
  lifted to `#c8c8c8`; right-aligned fused score mono 11 (`#9ee065` on the selected row, `#6b6b6b`
  otherwise). Selected row `#252525`.
- `1px #2a2a2a` divider, then the create affordance: green `+` avatar, `New note “<query>”` in
  `#9ee065`, destination `baking/` mono 11 right.
- Footer keyhints: `↑↓ move` · `return open` · `tab titles / text` · `← back` · `esc close`

### 5. Notifications and OSD

Cards `400px`, `1px #333`, radius 8, `#1c1c1c`, `padding: 14px`, `display:flex; gap:12px`, with a
`3px` full-height accent bar (radius 2) on the left instead of an icon.

- **Success** — accent `#9ee065`: "Vault synced" sans 13.5/700, body mono 11.5 `#a0a0a0` 1.6,
  `brain · now` mono 10 `#5c5c5c`.
- **Conflict** — accent `#ff6f8d`: title `#ff6f8d` "Sourdough.md changed on disk", body "Your edits
  are still here. Keep them, or take the file.", then two buttons `28px`: primary `Take the file`
  (amber fill, `#1c1c1c` text, 700) and secondary `Keep mine` (`#212121`, `1px #3a3a3a`, `#c8c8c8`).
  This is the redesign of today's `AdwBanner` alert.
- **OSD pills** — `padding: 10px 14px`, same card shell: `Reading mode` + a `ctrl+e` keycap;
  `Embedding` + an `80×4` track (`#2e2e2e`) with a `#7ed0dc` fill and `128/500` mono 11 `#7ed0dc`.

---

## Interactions & Behaviour

These come from the existing app and must survive the redesign.

**Editing model (non-negotiable)**
- One text view. The note is always source and always styled; syntax characters carry an invisible
  attribute that is *removed* for the construct the caret is inside — both halves of a pair together,
  and revealed when the caret rests immediately before the opener or after the closer.
- Reading mode (`Ctrl+E`) reveals nothing at all and refuses edits; it is the same view with the
  caret taken away, so scroll position never moves. In reading mode a plain click follows a link.
- Re-scan is per line: a keystroke re-scans the caret's line and the one above; only an edit that
  changes the line's outgoing state (opening a fence, touching a frontmatter delimiter) escalates to
  a full re-scan.
- `Enter` in a list repeats indent and bullet, or the next number, or a fresh unticked checkbox;
  `Enter` on an empty item ends the list; `Backspace` removes the bullet.

**Links**
- `[[` opens completion over titles and aliases (same list as capture uses).
- `Ctrl+Click` or `Ctrl+Return` follows; following a dead link offers to create the note; renaming a
  note repoints every link that pointed at it.
- Clicking a tag filters the note list.

**Files and saving**
- Saves coalesce on a 2-second tick and go out `tmp → fsync → rename`; flush on note switch, close and
  shutdown. `Ctrl+S` is a no-op that flushes and confirms.
- Frontmatter keys the app does not understand round-trip byte for byte. Only `tags`, `aliases`,
  `created`, `updated` are interpreted.
- External changes are noticed by a per-directory file monitor and surface as the conflict
  notification above.

**Search**
- The header field filters the tree as you type (title first, then text, with the matching line under
  each result); `Enter` opens the top result, `Escape` restores the tree.
- The summoned search fuses BM25 over words with embeddings from a local llama.cpp server via
  reciprocal-rank fusion. **No server is a supported state**, not an error: search falls back to
  words alone and the status rail says so rather than warning.
- Vectors are cached outside the vault; a first pass over an existing vault is slow (~90s per 500
  notes), runs in the background, resumes if interrupted, and reports progress in the OSD pill and
  the status rail.

**Drag and drop**
- Drag a note onto a folder to move it (a file rename); drag a folder to move it whole. Drop target
  highlight is a fill, never a border, so rows do not resize under the pointer mid-drag.
- Dropping a file on the editor, or pasting an image, copies it into `attachments/` and embeds it.

**Keyboard**
`1`–`3` nav · `Ctrl+N` new note · `Ctrl+K` go to note · `Ctrl+Shift+F` search all text ·
`Ctrl+F` focus the sidebar search · `Ctrl+E` reading mode · `Ctrl+Space` insert menu ·
`Super+N` capture · `Shift+F10` row menu · `Ctrl+S` flush.

**Hover / active states** (not drawn in the mocks — derive them)
- Rows: rest transparent → hover `#212121` → selected `#252525`; active nav keeps `#384a48`.
- Secondary buttons: `#212121` → hover `#262626`; border stays `#3a3a3a`.
- Amber button: fill `#f0cf7a` → hover a touch lighter; text stays `#1c1c1c`.
- Focus ring: use the codebase's existing focus treatment; the design has no bespoke one.

## State Management

- `vault_root`, `notes[]` (id, title, folder, mtime, ctime, excerpt), `folders[]` with expanded state
  and counts, `tags[]` with counts
- `open_note` (id, body, dirty flag, caret offset), `reading_mode`, `parsed` (spans + markers +
  per-line state) — the scan is derived, cached per line, never persisted
- `query` + `results` for the sidebar filter; separate `search_mode` (titles | text) and `hits` for
  the summoned search, plus a "vectors pending" flag so late-arriving semantic results can refresh an
  open menu
- `backlinks` for the open note (id + the line the link was written on)
- `sync_state` (`synced <time>` | pushing | conflicted), `embedding_progress` (done / total)
- `alert` — at most one: changed-on-disk, write-failed, or vault-missing
- The vault is canonical and the index derived. Widgets emit intent; a single owner performs every
  file write.

## Assets

None. No icon set, no images, no fonts to ship:

- The only glyphs used are text characters (`v`, `>`, `+`, `/`, `⌕`, `●`, `↑↓`, `→`, `←`) and CSS
  shapes. Substitute the codebase's icon set if it has one; do not commission icons for this.
- Image placeholders in the mock are striped SVG stand-ins labelled `attachment · oven-spring.jpg` —
  real attachments render in their place.
- JetBrains Mono is loaded from Google Fonts in the prototype only. In the real app, use the user's
  configured monospace family (fontconfig `monospace` alias on Linux); the design assumes a mono with
  a distinguishable zero.

## Files

| File | What it is |
|---|---|
| `Brain Quattro v2.dc.html` | **The design.** The vault window plus capture, insert menu, note search, notifications/OSD. Build from this. |
| `Brain Today.dc.html` | Pixel-faithful recreation of the **current** GTK/libadwaita UI, for before/after comparison and to see exactly which behaviours the redesign preserves. |
| `Brain Quattro.dc.html` | An earlier exploration in the Catppuccin/Quattro-token palette, including three window-density options and shell-integration surfaces. Superseded — reference only. |
| `support.js` | Runtime the `.dc.html` files need to render. Keep it beside them. |
| `github.md` | Records the source repo (`mhagrelius/brain`, branch `main`) and a screen → source-file map. |

Source files the recreation and this document were derived from, in `mhagrelius/brain@main`:
`src/ui/window.rs`, `sidebar.rs`, `details_panel.rs`, `backlinks_panel.rs`, `palette.rs`,
`highlight.rs`, `editor.rs`, `tag_tree.rs`, `style.css`; and `mhagrelius/quill@main:src/lib.rs` for
the scanner's `Style`, `Marker`, `Format` labels and syntax.
