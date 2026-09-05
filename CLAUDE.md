# brain

A Markdown notebook for Omarchy: Qt 6 / Qt Quick, C++20. Owns the vault format (Markdown + frontmatter) that Familiar also reads, and talks to `brain-server` on the NAS for sync and shared vectors.

This branch (`omarchy`) replaced the Rust + GTK app with this one. The server (`brain-server`, Rust) still lives on `main` under `server/`; the wire format it speaks is pinned here by `tests/test_wire.cpp`.

## Commands

- `bin/build` → `build/brain`. Install: `cmake --install build --prefix ~/.local`. Package: `packaging/PKGBUILD`.
- `cd build && ctest --output-on-failure` — one QtTest executable per `src/core/` area plus the wire format. This is the gate.
- `bin/grab [dir]` renders every screen × state headless (`--demo --screen … --act … --grab file.png`); `./build/brain --info` proves the theme parsed. `OMARCHY_TEXT_SCALE=1.3` checks growth, `OMARCHY_THEME_DIR=/usr/share/omarchy/themes/<name>` another theme.
- `./build/brain --demo` opens a throwaway vault of sample notes; `--vault <dir>` opens a folder for one run. Both are scratch runs: a private config and cache, no single-instance socket, no network unless `BRAIN_OFFLINE` is unset.
- CLI verbs go to the running instance: `brain capture`, `brain search [titles|text]`, `brain sync`, `brain status`, `brain config <key> [value]`.
- Qt logs go to the journal unless `QT_FORCE_STDERR_LOGGING=1`.

## Layout

- `src/core/` — QtCore only, no widgets, no sockets: `scanner` (the quill port: spans + hideable markers), `frontmatter`, `note`, `vault`, `index`, `search` (fuzzy titles, substring text, BM25, RRF fusion), `semantic` (chunks, digests, the vector store, the catch-up plan), `sync` (three-snapshot plan, `gather`/`apply`), `tree`, `config`, `notebook`.
- `src/net/` — QtNetwork, blocking, worker threads only: `http` (a small HTTP/1.1 client over QTcpSocket), `embedder` (llama.cpp `/v1/embeddings`), `vaultserver` (brain-server's vault and vector routes).
- `src/backend.*` — the `App` singleton: owns the Notebook, the timers (2 s save tick, 5 s catch-up delay, 60 s sync), the watcher, the worker threads, and formats every row QML shows. `src/editor.*` — the editing model. `src/qml/` — the views.

## Rules

**`Notebook` is the only thing that writes a file or mutates the index.** QML emits intent through `App`; nothing in QML or `Editor` touches the vault. A notebook method returns what happened, not what to display.

**The editor's source is canonical; the display is derived.** `Editor` keeps the note's body, the caret, and a display↔source offset map. The TextEdit shows the display (syntax removed outside the caret's construct); an edit the TextEdit makes is mapped back onto the source through the map, then the display is rebuilt. Never write to the QTextDocument except through `syncDocument`, and never from inside `contentsChange` (it is deferred to the next event-loop turn for that reason).

**A sync pass is two halves on two threads.** `sync::gather` does the network and reads local files, on a worker; `sync::apply` does every local write on the main thread, where it knows which note is open and whether it is dirty. Keep new sync behaviour on the matching side.

**Workers hop back with `Backend::runOnMain`**, which checks the static instance under a mutex and queues onto the main thread. Results carry the `m_generation` they were started under; a vault switch bumps it and stale results are dropped.

**u64 on the wire.** Hashes and digests are FNV-1a u64 and exceed a JSON double. Bodies are built by hand (`VaultServer::putBody` etc.) and replies pass through `json::quoteBigIntegers` before parsing. Change the format on either side and `test_wire` fails, which is the only warning there is.

**Frontmatter is not rendered.** The editor holds the body; the rail edits tags and aliases. Unknown keys round-trip byte for byte — `test_frontmatter` asserts it.

Sizes only through `T.s()` / `T.f()`; colours only through `T.<role>` (derived in `src/palette.cpp` from colors.toml — the handoff's greys are surface roles, its amber is `accent`, green `positive`, cyan `teal`, pink `negative`). Read the `omarchy-app-dev` skill before changing theming, scaling or packaging. The design handoff is in `docs/handoff/`.
