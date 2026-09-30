# Prompt: bring brain onto the shared design language

You are Claude (Fable 5.1) working in `~/Projects/brain`. Brain was built from a
handoff written in ledger's language but with its own numbers: a 62-tall header,
32-tall controls, weight 700, twelve type sizes, a system sans face. The shared
spec has one control height, two button sizes, eight type sizes and IBM Plex Sans.
Bring the app to it exactly. Change how things look, never what they do: the
scanner, the editor's display ↔ source map, the notebook, sync, search, the wire
format and every test are out of scope. The editing model in `CLAUDE.md` is
non-negotiable.

## Read first, in this order

1. `/home/matthew/Projects/design/DESIGN.md` — the spec. §3 (type), §4 / §4a / §4b
   (geometry, spacing, layout), §5 (components), §9 (the divergence audit; the
   `brain` column is what this app does today, the last column is the target).
2. `/home/matthew/Projects/design/sheet.html` — the spec as pixels. Render it
   headless if you cannot open a browser:
   `chromium --headless=new --disable-gpu --hide-scrollbars --allow-file-access-from-files --window-size=1440,16200 --screenshot=/tmp/sheet.png file:///home/matthew/Projects/design/sheet.html`
   and crop with `magick`. `tokens.css` there is the CSS form of every size below.
   Layout L4 is this app.
3. `/home/matthew/Projects/design/screens/brain/` — every screen of this app as it
   is now, rendered under the Monokai Pro theme. Your "before".
4. This repo's `CLAUDE.md`, `docs/handoff/README.md`, and the `omarchy-app-dev`
   skill.

If this checkout is not at `~/Projects/brain` next to `~/Projects/design`, copy
`DESIGN.md`, `sheet.html`, `tokens.css`, `theme.js` and `fonts/` from the design
folder into `docs/design-language/` here and read them from there.

## Invariants

- In scope: `src/qml/**`, `T.qml`, `palette.cpp` / `.h`, `main.cpp` (fonts),
  `CMakeLists.txt` (font resources), `packaging/` (font files). Out of scope:
  `src/core/**`, `src/net/**`, `backend.*`, `editor.*`, tests.
- Every size through `T.s()` / `T.f()`, every colour through `T.<role>`. No hex.
- No new sizes. After this work, `grep -rhoE 'px: *[0-9.]+' src/qml | sort -u`
  must print only `10.5 11.5 12.5 13.5 14.5 15 18 21 24 25` (14.5 / 18 / 24 are the
  editor's prose and headings and appear only in the editor). Two button sizes, one
  control height. If something genuinely needs another value, write down why in
  the decisions file before adding it.
- Record every departure from the spec, and the decision below, in a new
  `DECISIONS.md` at the repo root (use planner's as the model). Do not edit
  `DESIGN.md` here; it is the product design, not the visual one.

## Changes, in order

Fonts, tokens and components first (1–5), rebuild, re-grab every screen, then the
screens (6–9). One commit per numbered step.

1. **Sans face.** Bundle IBM Plex Sans exactly as ledger does: copy
   `~/Projects/ledger/fonts/` (Regular, Medium, SemiBold, the OFL) into `fonts/`,
   add them to the QML resource in `CMakeLists.txt`, register them in `main.cpp`
   with `QFontDatabase::addApplicationFont`, set the application font, and call
   `palette.setSansFamily("IBM Plex Sans")`. Mono stays the `monospace` alias.
   Record the decision (the handoff said Helvetica / system; the owner's call is
   Plex in all four apps).
2. **Type ramp and weight.** Map every `px:` onto the roles (DESIGN.md §3):
   - 9.5, 10, 10.5, 11 (mono: counts, mtimes, keyhints, status rail, rail keys,
     card labels, footers) → **10.5**.
   - 11.5 mono (toast body) → **sans 12.5** (sentences are sans); 12 mono (rail
     values, syntax column) → **12.5**; 12.5 inline code stays.
   - 13 / 13.5 sans (tree rows, nav rows, menu rows, search placeholder, buttons)
     → **12.5** for nav / tree rows, fields and buttons; **13.5** for menu and
     search-result rows (they are list rows) and card headings (Backlinks).
   - 14 (card titles, capture header) → **13.5**; 15 header title stays; 16 "Brain"
     → **15**.
   - Prose 14.5 / 1.8, H1 24, H2 18 stay; they are the editor's.
   - Every `Font.Bold` → `Font.DemiBold` (eight uses: app name, folders, active nav,
     headings, note H1 / H2).
   - `Eyebrow.qml`: mono, 10.5, uppercase, `T.r(10.5 * 0.16)`, `muted`; route the
     rail's inline uppercase labels (`THIS NOTE`, `VAULT`, `RECENT`) through it.
3. **Controls: one height.** `PrimaryButton` / `SecondaryButton`: explicit
   `height: T.s(28)`, sans 12.5, side pad 12, radius 6, primary 600; add
   `small: true` (24, sans 11.5, pad 8). Remove every override: the 32-tall /
   px 13 / padX 14 instances in `NotePane`, `PromptPopup`, `StatusPanel`,
   `FirstRun`, and the 12.5 / 12 / 28 ones in `Toasts`. `Field` 28 tall (`padY` 5);
   the toolbar search field 28 (was 250 × 32; keep 250 wide). The capture and
   search prompt fields are the one exception at **32** tall, sans 13.5, with the
   mono sigil.
4. **Small parts.** `Keycap.qml`: mono 10.5 (was 10), colour `faint` (was
   `dimmest`), 4 r and `borderStrong` stay. Tag chips: mono 10.5, 18 tall, 4 r,
   `teal` on `tealBorder`. Avatar squares 26 / 6 r with mono 10.5 / 600 on
   `neutralBg`, no border (one avatar style across the apps). Secondary buttons
   are transparent everywhere: drop the `fill: T.card` / `borderStrong` overrides
   in Toasts and StatusPanel. `Tooltip`:
   title 12.5 / 600, sub and hint 10.5. `Popover.qml`: `card` fill (was `window`),
   `borderStrong` (was `border`), 8 r stays; replace the three feathered shadow
   rectangles with one (the spec has one shadow, `0 18 40 / .5`; a single
   `MultiEffect` or one rectangle at .5 is fine). Menu rows 30 stay.
5. **Palette.** Brain's `hintText` / `meta` / `dimmest` derivation is the one the
   other apps will adopt; keep it. Where `dimmest` colours keycaps or footers, use
   `faint` instead (`dimmest` remains available for the editor's revealed markers
   only).
6. **Sidebar.** App name sans 15 / 600 over the mono 10.5 vault line (was 16 / 700
   + 11). Nav rows 32 with the 13 number slot, active row `activeFill`, label 600
   (not 700). Tree rows 32; child indent 37 stays. Section label row 14 / 16 / 6.
   Footer 26 on `header` fill, mono 10.5 `faint`, one keyhint left in lowercase
   (`ctrl+k search`) and `1–3` right; the insert hint moves to the status rail.
7. **Content header.** 54 tall (was ~62): 14 / 20 padding is gone; the title
   (15 / 600) and the mono 10.5 subtitle stack on the left like ledger's toolbar,
   the 28-tall search and the 28-tall "+ New note" sit right at a 12 gap.
8. **Rail and status rail.** The right rail stays 360 with 16 pad and cards 14
   apart; card body 14 / 15; the "This note" key / value rows 26 tall, keys mono
   10.5 `muted`, values mono 12.5. Backlinks heading 13.5 / 600 with a mono 10.5
   count; entries at an 11 gap from the avatar. The status rail is **26** (was 28),
   `header` fill, mono 10.5 `faint`, "synced hh:mm ●" right.
9. **Summoned surfaces and toasts.** Capture 560, insert 520, search 660,
   centred at 64 from the top, `card` fill, `borderStrong`, 8 r, one shadow, over
   the one `rgba(0,0,0,.5)` scrim (brain has none today; add it, and do not dim
   the shell otherwise); the prompt field inside is a boxed 32-tall field with
   `window` fill (a field is one step from its surface); header 12 / 14, rows 30 at 0 / 14,
   footer 28 with keyhints 16 apart. Toasts: 400, bottom-right 16 in and 10 apart,
   3 px accent bar, sans 13.5 / 600 title, **sans 12.5 body**, mono 10.5 source
   line, default-size buttons 6 apart. OSD pills: the card shell at 10 / 14, label
   13.5 / 600.
10. **Screenshots and docs.** `bin/grab docs/screens`; add a "Design language"
    paragraph to `README.md` pointing at `~/Projects/design/DESIGN.md`; note in
    `docs/handoff/README.md` that the handoff's sizes are superseded by the spec.

## Decision to assume unless the owner says otherwise

- Bundle IBM Plex Sans (the handoff asked for the system sans). Everything else
  in this prompt is the spec's recommendation, not a choice.

## Verify

```sh
bin/build && (cd build && ctest --output-on-failure)
QT_FORCE_STDERR_LOGGING=1 QT_QPA_PLATFORM=offscreen BRAIN_OFFLINE=1 timeout 4 ./build/brain --demo   # exit 124, empty stderr
./build/brain --info                                                                  # sans: IBM Plex Sans
bin/grab docs/screens
OMARCHY_TEXT_SCALE=1.3 bin/grab /tmp/scale
OMARCHY_THEME_DIR=/usr/share/omarchy/themes/catppuccin-latte bin/grab /tmp/light
grep -rhoE 'px: *[0-9.]+' src/qml | sort -u                                           # only the listed sizes
grep -rn 'Font.Bold' src/qml                                                          # empty
```

Compare each new screenshot with `~/Projects/design/screens/brain/<screen>.png`
(before) and with the sheet's L4 and L5 frames (target). Then, if `hypruse` is
available, launch the installed build, open a note, type in it, open capture,
insert and search, trigger the conflict toast (`--act conflict` shows what it
looks like), and check every row of controls aligns at one height and that the
editor still hides and reveals syntax exactly as before.

## Report

Finish with: the list of `px` values before and after, every place a size or
spacing changed that the spec did not anticipate (with the value you chose and
why), the DECISIONS.md entries, and the before / after screenshots side by side.
