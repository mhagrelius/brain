# Brain beyond GNOME — plan

Brain is the test case for two changes that would apply to the sibling apps if
they work here: a core that is not tied to GTK, and a vault that reaches more
than one machine. This is the order to do them in and the reasoning that picked
it.

**Where this has got to.** Phases 0 to 3 are done and on `main`. Phase 4 has
its two halves that can be tested in isolation — the planner in
`core/src/sync.rs` and the service in `server/src/notes.rs` — and does not yet
have the part that joins them: the client transport, the notebook's sync pass,
and the conflict banner. Nothing in Brain calls any of it yet, which is
deliberate; a half-wired sync is the one thing here that could lose a note.

`DESIGN.md` still describes what was built and why. Where this plan contradicts
it — sync, most obviously, which `DESIGN.md` calls somebody else's problem —
the difference belongs in that file's "Built differently, or not built" section
once it is real, not as an edit to the design.

## What does not change

The vault is canonical and notes are ordinary files. Every client keeps a full
local replica, not a remote mount: `gio::FileMonitor` is inotify and does not
propagate over NFS or SMB, and a 1,000-note scan that costs 1.8 ms locally is
1,000 network round trips over a share. A server that holds the only copy is a
reason for the notes to be unreachable, and there is no version of that worth
having.

The server stores and arbitrates. It does not become the place the notes live.

## Why the core comes out first

`ui/application.rs` is 1,728 lines with 35 GTK references — one per 49 lines.
Its contents are `create_note_in`, `rename_note`, `move_note`,
`relocate_folder`, `sidebar_rows`, `search`, `catch_up_now`. That is a headless
client already written; it is wearing a `glib::wrapper!` because that was the
convenient place to hang it. `ui/embedder.rs` is 2 references in 351 lines and
is core code filed in the wrong directory.

Against that, `ui/sidebar.rs` is one reference per 8 lines and `ui/window.rs`
one per 12. Those are genuinely GTK and will be rewritten per platform.

The split is therefore about 7,000 lines of core against 6,000 of shell, not
the 5,000/8,600 that the directory names suggest.

The payoff arrives before any second shell exists. `tests/session.rs` is 581
lines with no GTK at all; `tests/lifecycle.rs` is 819 lines that need Xvfb and
the one-`#[test]`-over-`STEPS` contortion only because the policy it exercises
lives inside a GObject. Moving that policy frees 819 lines of scenario coverage
from the display harness.

## Phases

### 0. Workspace split

`brain-core` (no GTK) and `brain-gtk`. Move `model/` and `ui/embedder.rs`
across unchanged. Mechanical and reversible; touches every import and the
packaging scripts, so it lands as its own commit with no behaviour change.

### 1. Lift `Notebook` out of `BrainApplication`

A plain struct in core owning vault, index, store, dirty set and open note.
`BrainApplication` keeps a `RefCell<Notebook>` and retains only what is
genuinely GTK: actions, dialogs, toasts, timers, file monitors, and telling
widgets to redraw.

The invariant survives verbatim — `Notebook` becomes the only thing that writes
a file or mutates the index. Same rule, different type.

Take values out of the `RefCell` before any `match` scrutinee. This refactor is
precisely where that bites.

Done when the steps of `tests/lifecycle.rs` that do not need a widget run as
ordinary `cargo test`. A step that cannot move means the boundary is in the
wrong place, found for the price of a test file rather than a Swift app.

### 2. Make external change a real choice

`absorb_external_changes` does not do what `DESIGN.md` claims. The design says
the banner offers reload or keep; the code shows a bare warning — "changed on
disk. Saving will overwrite that." — with no button (`application.rs:501`).

That is tolerable today because external changes are rare and usually your own
`git checkout`. Under sync, a note changing while you type it is routine, and
warn-then-overwrite stops being good enough. The banner needs real actions
before anything syncs.

`set_save_error` is one banner already shared between save failures and
external-change warnings. Conflicts make three conditions on one surface, so it
needs a priority rule: an active save failure outranks everything, because that
is data not being written right now.

Valuable on its own, independent of the server.

### 3. Vectors on the server, before notes

Stand the NAS service up on the disposable thing first. A broken vector service
means search is lexical-only for a while; a broken note service means lost
notes. Prove the container, the Tailscale path and the client's tolerance of an
unreachable server against the failure that costs minutes, not the one that
costs work.

`model::semantic::Store` is already named in `DESIGN.md` as the seam where this
plugs in — one type to reimplement. Embed once on the NAS, every client pulls.
This also removes the current duplication of embedding the same vault three
times on three machines.

### 4. Sync — half built

Done: `core/src/sync.rs`, the planner, with the three-snapshot comparison and
both rules below; and `server/src/notes.rs`, the vault as real Markdown with
stale writes refused. Both are tested, and the service is verified over real
HTTP — a stale write comes back 409 with the current hash, and a note id of
`../escaped.md` is refused without touching the filesystem.

Still to do, and none of it started:

- **The client transport.** `src/ui/sync_client.rs`, the same shape as
  `shared_vectors.rs`: soup, on the worker thread, behind a trait in the core.
- **The base snapshot.** The planner needs what the two sides agreed on last
  pass, kept in `.brain/` beside the vectors. Losing it is not fatal — an
  empty base makes a first pass, which pushes and pulls everything and calls
  nothing a conflict that is not one — but it has to be written somewhere.
- **The pass itself**, on `Notebook`, applying a `Plan`: the ordering matters
  and is not obvious. Renames before edits, deletions last, and the base is
  only updated for the notes whose transfer actually succeeded, so a pass that
  dies half way through is behind rather than wrong. That is the same promise
  the embedding catch-up makes.
- **The conflict UI**, which is one banner string and one sidebar filter mode,
  per the notes below.

Vault on the NAS as real Markdown files in a container, git-backed for history,
readable by familiar and by `cat`. The service exposes per-note content hashes,
a "changed since version X" endpoint, and a write that is refused if its base
hash is stale.

`model/sync.rs` in core is a pure planning function taking local state and
remote state and returning work — the shape `semantic::plan` already
established, with its own tests and no display. It runs on a timer on a worker
thread, is expected to lag, and nothing on screen waits for it.

**A conflict is a note, not a question.** The divergent version is written into
the vault as `Rust ownership (conflict 2026-08-04 from phone).md`:

- It survives brain being closed, crashed or uninstalled, which a dialog answer
  does not — and a modal that appears mid-typing is dismissed reflexively,
  which is exactly when it costs a note.
- It appears in the sidebar already, because the sidebar is the directory tree.
- It is resolved with what exists: open both, reconcile, delete the loser —
  and delete already carries a toast with undo.
- A different title cannot steal `[[Rust ownership]]` links or trip the
  same-title ambiguity report.
- The filename carries the provenance, so frontmatter stays frozen at four
  keys.

Two structural cases are rules rather than questions. **Deletion never wins
over an edit** — losing a note is the one unrecoverable failure, the same
reasoning `DESIGN.md` applies to stale vectors. **Rename and edit compose** —
links resolve by title and rename already rewrites inbound links.

UI is awareness, not arbitration. An unresolved conflict is an ongoing
condition, which by the toast → banner → dialog escalation puts it on
`AdwBanner`: "3 notes have conflicting copies", with a button that filters the
sidebar to them. The sidebar already filters for tags and for search, so that
is wiring rather than new UI. `GNotification` if the window is backgrounded.
Net new UI is one banner string and one sidebar filter mode.

No merge editor. A three-way text merge is custom drawing on the scale of the
graph view that `DESIGN.md` declined, for a conflict rate nobody has measured.
The cheap substitute is showing both versions in the split view that already
exists for backlinks. Measure how often it bites before paying for Automerge.

### 5. macOS shell

UniFFI with proc-macros rather than UDL, `cargo build` → XCFramework → SPM
binary target, SwiftUI. Well-trodden: it is Mozilla's own use case and the
route every Rust-core-plus-native-UI project in the survey took.

The FFI surface gets designed here, against a real consumer. Designing it
during phase 1 would mean designing against an imaginary one.

The editor is less frightening than it looks. `quill` already emits spans and
markers in char offsets and `ui/highlight.rs` is the policy that turns those
into tags; `NSTextView` takes attributed ranges the way `GtkTextBuffer` takes
tags. The port is the same spans applied to a different text system, not a new
editing model.

### 6. Windows, and mobile

Deferred deliberately, and decided after phase 5 rather than now.

Windows is the weak leg: `uniffi-bindgen-cs` is third-party, so if it proves
flaky the fallback is a hand-written C ABI with P/Invoke, or a serialized
message boundary that any language can host. Prototype the binding before
committing to WinUI 3.

Mobile is where the core/shell split pays off most, and it is also the last
thing to build. In the meantime the vault is real Markdown on a synced
filesystem, so **Obsidian mobile reads it today** — free, and the payoff of
"the vault is the product".

## Do not ship GTK on Windows or macOS

It builds. It is also, from the current GNOME bug reports and discourse
threads: renderer artifacts on AMD, no window snapping or tiling, drag-and-drop
freezes, poor font rendering, oversized shadows that swallow clicks, and
animation starving the main loop on Win32. libadwaita has no ambition to look
native off GNOME, and that is a stated position rather than a gap.

One first-class GNOME app and two mediocre ports is the opposite of the goal.

## Packaging

| Platform | Artifact | State |
|---|---|---|
| Linux | Flatpak, deb, `install.sh` | Exists |
| Core | staticlib + XCFramework | Phase 5 |
| macOS | `.app` in a `.dmg`, or a Homebrew cask | Phase 5 |
| Windows | MSIX or an installer | Phase 6 |
| Server | container image on the NAS registry | Phases 3–4 |

`test.sh` stays the gate for core and the GTK shell. The Swift and C# shells
get their own, and the core's tests run once rather than per platform — which
is the whole point.

## Deliberately not decided yet

Whether any of this generalises to familiar, planner, stickies and the rest.
Brain goes first precisely so that the pattern is written down from something
that shipped rather than from a survey. A skill codifying it comes after phase
5, not before.
