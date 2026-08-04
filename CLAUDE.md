# brain

A Markdown notebook. Owns the vault format (Markdown + frontmatter) that Familiar also reads.

## Stack

GTK 4.22 + libadwaita 1.9 via gtk4-rs 0.11 / libadwaita-rs 0.9, Rust edition 2021 (MSRV 1.80). `gio` is a direct dependency purely to raise the API level to v2_80 — leave it.

Cargo workspace of two crates. `brain-core` (`core/`) is the vault, notes, index and search, and links no UI toolkit at all — not even GLib. `brain` (the root package) is the GTK shell, a lib + bin so integration tests and `examples/` can drive the real application rather than a copy of it. `src/lib.rs` re-exports the core as `brain::model`, so every `model::…` path reads as it always did.

See `PLAN.md` for where this is going: the split exists so a second shell on another platform can keep the core.

## Commands

- `./test.sh` — fmt check, clippy with `-D warnings`, then `cargo test --workspace --all-targets`. Add `--headless` to run under Xvfb + a private D-Bus session. This is the gate; run it, not bare `cargo test`. **`--workspace` is not optional**: without it cargo tests only the root package, and the whole of `brain-core` — the half that needs no display — silently does not run.
- **Never run `dbus-run-session` or `xvfb-run -a dbus-run-session` directly** — use `isolated-bus [--headless] -- CMD`. A private bus activates its own `xdg-document-portal`, which mounts over `/run/user/$UID/doc` and takes the login session's portal down with it when the bus exits; every flatpak on the machine then fails to launch until it is restarted. `test.sh --headless` guards against this internally, but one-off runs of a single test, or of the built binary, bypass it.
- `cargo run --example preview -- <dir> [dark]` — renders the real widgets to PNG offscreen. This is how a UI change is looked at; screenshotting a live Wayland session needs interactive consent. Run it under `xvfb-run -a`.
- `cargo run --example icons_check` — asserts every icon name resolves. An unresolved one draws a missing-image glyph and warns about nothing, so add new names to `USED`.
- `./install.sh` — release build, installs under `~/.local`. `./uninstall.sh` reverses it.
- `packaging/build-flatpak.sh` and `packaging/build-deb.sh` — distribution artifacts.
- `./sync-check.sh` — starts a throwaway `brain-server`, drives it with the real client, and takes it down. The only check that catches a client and a server which are each self-consistent and disagree. Not part of `test.sh`, because it wants a port and a build of both.
- `server/` is `brain-server`: the shared vector store, and the vault as real Markdown files. `podman build -f server/Containerfile -t brain-server .` from the repo root — the context must be the workspace, since it depends on `brain-core` by path. It needs `BRAIN_VECTORS_TOKEN` (32 characters or more) and refuses to start without one. **`notes::path_of` is the only thing between a note id off the network and the server's filesystem** — it allows rather than forbids, and its test table is the specification.

## Layout

`core/src/` is pure logic with no GTK types. `src/ui/` is widgets and the application. Read `DESIGN.md` and `README.md` before proposing structural changes; both are current.

**`Notebook` (`core/src/notebook.rs`) is the only thing that writes a file or mutates the index.** Widgets emit signals of intent and change nothing themselves, so there is exactly one place a note can be lost. `BrainApplication` holds it in a `RefCell` and does what only a toolkit can: actions, the save tick, file monitors, worker threads, and turning an outcome into a toast. Keep new behaviour on the notebook's side of the line.

**A notebook method returns what happened, not what to display.** `Renamed::Done { links }` carries the count; the sentence about it belongs to the shell, because a second shell will word it differently. That is also what the tests assert on.

**Push logic down into `brain-core`.** A rule that lives there is tested by `cargo test` with no display; the same rule inside a widget is only reachable through the GTK harness. The sidebar's tree is the worked example — `core/src/tree.rs` decides what the rows are, and the widget only draws them.

**The embedder stayed in the shell on purpose.** `semantic::Embedder` is a trait in the core; `src/ui/embedder.rs` is libsoup's answer to it. Keeping the transport out of the core is what stops GLib being dragged onto a platform that has no use for it — anything new that opens a socket goes on the shell side of that trait too. `semantic::Shared` and `src/ui/shared_vectors.rs` are the same pattern for the shared vector store.

**A sync pass is two halves and they run on different threads.** `sync::gather` does the network and reads local files but writes none, on the worker; `sync::apply` does every local write on the thread that owns the notebook. This is not the catch-up's shape and must not be rewritten into it — the catch-up is handed copies and gives back a new store, while a sync writes files the save tick is also writing. New sync behaviour goes on whichever side matches: reads and network in `gather`, writes in `apply`.

**Wire formats are pinned on both sides.** The client and `brain-vectors` define the same JSON in two crates that never see each other's types, so each has a test asserting the exact bytes. Change one and the other's test fails, which is the only warning there is.

## Testing

Widget tests need a display; model tests do not and are the bulk of the suite. `test.sh` sets `GTK_A11Y=none` and `GSETTINGS_BACKEND=memory` so tests never touch real user state — keep that true for anything new.

GTK is thread-affine, so `tests/widgets.rs` and `tests/lifecycle.rs` are each **one `#[test]` over a table** — `CASES` and `STEPS`. Add a case to the array; a second `#[test]` that touches GTK will fail. `lifecycle.rs` steps run in order against one shared vault, so a step that leaves a note behind breaks a later assertion.

`core/tests/notebook.rs` has none of those constraints — a plain `#[test]` per scenario, no display, no shared vault. **A new rule about what happens to the vault goes there, not in `lifecycle.rs`.** Reserve `lifecycle.rs` for what genuinely needs a widget in the loop.

## Conventions

- Use the `developing-gtk-apps` and `designing-gnome-ui` skills for widget, threading, and HIG decisions rather than deriving them again.
- Edit files with the Edit tool. Do not rewrite Rust sources through `python3 - <<PY` heredocs or `sed -i` — including test files.
- **Take a value out of a `RefCell` before a `match` or `if let` scrutinee.** The borrow lives for the whole body, and a `replace` inside it panics at runtime with nothing at compile time to warn you. This has cost real debugging more than once.
- When behaviour ends up differing from `DESIGN.md`, add it to that file's "Built differently, or not built" section rather than editing the design to match. The record of why is the point.
- The sibling apps (familiar, planner, stickies, youtube-downloader) share this layout and these scripts; a pattern established in one is the pattern here.
