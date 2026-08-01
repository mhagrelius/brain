# brain

A Markdown notebook. Owns the vault format (Markdown + frontmatter) that Familiar also reads.

## Stack

GTK 4.22 + libadwaita 1.9 via gtk4-rs 0.11 / libadwaita-rs 0.9, Rust edition 2021 (MSRV 1.80). `gio` is a direct dependency purely to raise the API level to v2_80 — leave it.

Crate is a lib + bin so integration tests and `examples/` can drive the real application rather than a copy of it.

## Commands

- `./test.sh` — fmt check, clippy with `-D warnings`, then `cargo test --all-targets`. Add `--headless` to run under Xvfb + a private D-Bus session. This is the gate; run it, not bare `cargo test`.
- `cargo run --example preview -- <dir> [dark]` — renders the real widgets to PNG offscreen. This is how a UI change is looked at; screenshotting a live Wayland session needs interactive consent. Run it under `xvfb-run -a`.
- `cargo run --example icons_check` — asserts every icon name resolves. An unresolved one draws a missing-image glyph and warns about nothing, so add new names to `USED`.
- `./install.sh` — release build, installs under `~/.local`. `./uninstall.sh` reverses it.
- `packaging/build-flatpak.sh` and `packaging/build-deb.sh` — distribution artifacts.

## Layout

`src/model/` is pure logic with no GTK types. `src/ui/` is widgets and the application. Read `DESIGN.md` and `README.md` before proposing structural changes; both are current.

**`BrainApplication` is the only thing that writes a file or mutates the index.** Widgets emit signals of intent and change nothing themselves, so there is exactly one place a note can be lost. Keep new behaviour on that side of the line.

**Push logic down into `src/model/`.** A rule that lives there is tested by `cargo test` with no display; the same rule inside a widget is only reachable through the GTK harness. The sidebar's tree is the worked example — `model/tree.rs` decides what the rows are, and the widget only draws them.

## Testing

Widget tests need a display; model tests do not and are the bulk of the suite. `test.sh` sets `GTK_A11Y=none` and `GSETTINGS_BACKEND=memory` so tests never touch real user state — keep that true for anything new.

GTK is thread-affine, so `tests/widgets.rs` and `tests/lifecycle.rs` are each **one `#[test]` over a table** — `CASES` and `STEPS`. Add a case to the array; a second `#[test]` that touches GTK will fail. `lifecycle.rs` steps run in order against one shared vault, so a step that leaves a note behind breaks a later assertion.

## Conventions

- Use the `developing-gtk-apps` and `designing-gnome-ui` skills for widget, threading, and HIG decisions rather than deriving them again.
- Edit files with the Edit tool. Do not rewrite Rust sources through `python3 - <<PY` heredocs or `sed -i` — including test files.
- **Take a value out of a `RefCell` before a `match` or `if let` scrutinee.** The borrow lives for the whole body, and a `replace` inside it panics at runtime with nothing at compile time to warn you. This has cost real debugging more than once.
- When behaviour ends up differing from `DESIGN.md`, add it to that file's "Built differently, or not built" section rather than editing the design to match. The record of why is the point.
- The sibling apps (familiar, planner, stickies, youtube-downloader) share this layout and these scripts; a pattern established in one is the pattern here.
