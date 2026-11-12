# Rustwalnut build plan

Rustwalnut is an Obsidian-vault-compatible knowledge base written in Rust. This file is the
build order and the status ledger. `DECISIONS.md` holds the tradeoffs and measurements.
Status values are honest: **done** means implemented with passing tests, **partial** means usable
with listed gaps, **stub** means the interface exists but not the behavior, **not started** means
exactly that.

## Layering (nothing is built on a layer without passing tests)

```
L0  md-engine      theme-engine (CSS)      mermaid-core
L1  vault-core  (uses md-engine)           editor-core (uses md-engine)
L2  search-core    graph-core    canvas-core    bases-core      (use vault-core)
L3  ext-api  (commands, events, feature registry, config schemas, scripting)
L4  plugin-host-wasm        compat-js (optional, separate cdylib)
L5  walnut-ui  (retained widget tree, taffy layout, CSS styling, render backends)
L6  app  (shell, CLI, IPC, settings, hotkeys, URI handler, packaging)
```

## Build order

| # | Step | Gate to move on |
|---|------|-----------------|
| 0 | Toolchain, workspace, lint config, three risk spikes | spikes measured, recorded in DECISIONS.md |
| 1 | `md-engine`: block + inline parser, Obsidian extensions, incremental region reparse | unit + proptest + snapshot tests |
| 2 | `vault-core`: atomic IO, metadata cache, link resolution, rename propagation, persisted index, watcher, trash, snapshots | unit + proptest (link rewriting) |
| 3 | `search-core`: query language with every Obsidian operator, index, fuzzy switcher | operator tests, budget bench |
| 4 | `graph-core`: graph build, filters, groups, Barnes-Hut layout, `graph.json` | construction tests, layout bench |
| 5 | `editor-core`: rope, multi-cursor, undo tree, folding, decorations, vim, smart lists, tables | unit tests, typing bench |
| 6 | `theme-engine`: CSS subset parser, cascade, variables, media queries | parser + cascade tests, fuzz target |
| 7 | `canvas-core`, `bases-core`, `mermaid-core` | round-trip + snapshot tests |
| 8 | `ext-api` + scripting + `plugin-host-wasm` | host/guest tests, limits tests |
| 9 | `walnut-ui` + `app` | headless render snapshot tests, CLI tests |
| 10 | `compat-js`, compat harness, community browser | harness output in PLUGIN_COMPAT.md |
| 11 | Perf-budget job, CI, packaging, docs, examples | CI config + local bench results |

## Status ledger

State at the end of the first build session. See the README feature checklist for the per-feature view.
Steps 0–6 of the build order are complete; step 7 is one third done (canvas only); steps 8 and 10
were not started; steps 9 and 11 are partial.

| Crate | Status | Notes |
|-------|--------|-------|
| spikes | done | results in DECISIONS.md §2–4 |
| md-engine | done | 54 unit/render tests, 5 property tests (incremental == full reparse), 2 snapshots. CommonMark-oriented, not run against the CommonMark spec suite. Full parse 30 MB/s; re-parse per keystroke 78 µs at 1 MB, 630 µs at 5 MB |
| vault-core | done | 26 unit tests + rename property test (40k cases: every link resolves to the same file after a rename). Warm open 13 ms / 25 ms / 130 ms at 5k / 10k / 50k notes; first index 47 ms / 87 ms / 433 ms. System trash is XDG-only (Windows falls back to `.trash/`). |
| search-core | done (native backend) | Every Obsidian operator, boolean/grouping, regex, properties; 14 test groups. 50k notes: 3–29 ms per query, quick switcher 1.5–4.7 ms. The `tantivy` backend is **not implemented** (DECISIONS.md §6). Embedded `query` blocks are rendered by the app through this API. |
| graph-core | done (headless part) | Global/local graph, depth, all filters, color groups, forces, pin, `graph.json` round trip, layout disk format; 10 tests. Barnes-Hut tick 3 / 5 / 26 ms at 1k / 10k / 50k nodes. GPU instanced rendering and LOD labels belong to the UI layer. |
| editor-core | done (model) | Rope buffer with incremental parse, transactions, undo tree with branches, multi-cursor, smart lists, auto-pair, indent, formatting toggles, folding, source/live-preview decorations, table grid editing, vim subset (documented in `vim.rs`); 30 unit tests + property test. Keystroke: 55 µs at 1 MB, 273 µs at 5 MB (edit + re-parse + viewport decorations). Spellcheck is a hook only; drag/drop and paste of files are UI-layer work. |
| theme-engine | done (engine) | Parser, selectors, cascade, `var()`, `calc()`, media queries, shorthands, transitions (parse + easing), inspector; 15 unit tests + never-panic property tests (40k cases). Supported/unsupported subset is listed in the crate docs. Loading `.obsidian/themes` and snippets with live reload is app-layer work. |
| canvas-core | done (model) | JSON Canvas parse/edit/write with lossless round trip (unknown fields, key order, Obsidian's layout); 3 tests. No canvas view. |
| bases-core | not started | Crate does not exist. |
| mermaid-core | not started | Crate does not exist. No Rust Mermaid crates were evaluated. |
| ext-api | partial | Versioned (API v1) JSON call boundary (`Api::call`), `Host` trait implemented by the app (`crates/app/src/plugins.rs`), per-plugin permissions (`vault.read/write`, `editor.read/write`, `ui`), vault-confined paths (dot-folders, `..`, absolute paths and symlink escapes rejected), commands, status-bar/ribbon/notices, per-plugin `data.json`, events (file-open, modify, create, delete, rename, layout-ready), discovery of `.obsidian/plugins/*`, a failing-plugin circuit breaker. 8 tests + 3 app tests. Core features are still not modules behind it, and plugin hotkeys are not read from `hotkeys.json`. |
| plugin-host-wasm | done (host) | `wasmi` 2.0 (pure Rust, `portable-dispatch`). ABI in the crate docs: exports `alloc`/`on_load`/`on_event`/`run_command`/`on_unload`, import `env.host_call`. Fuel budget per call, wall-clock deadline checked at host calls, 32 MB memory cap, host-call count cap, module size cap; 7 limit/permission tests with hand-written WAT, plus a real Rust example (`examples/plugins/hello-wasm`, 1 KB). Loaded lazily on the second frame; startup to first frame unchanged (30.8 ms headless at 10k notes). Release binary 6.5 MB -> 8.9 MB with the `wasm-plugins` feature (default). A runaway loop with no host calls is bounded by fuel only, not by the clock. |
| compat-js | not started | Only the engine spike (DECISIONS.md §3). |
| walnut-ui | partial | Element tree, CSS styling, own flexbox subset (taffy was dropped, DECISIONS.md §7), text shaping/caret/hit queries, software rasterizer; 3 tests incl. pixel checks. No wgpu backend, no accesskit, no transitions applied, no visual-regression snapshot suite. |
| app | partial | Window (winit + softbuffer), workspace view, live preview editor, explorer, tabs, palette, switcher, search panel, outline/backlinks, hotkeys, autosave, CLI (`index search export doctor screenshot commands`); 6 integration tests. See the README checklist for what is missing. |

## Known constraints of the build machine

The machine this was developed on has no C compiler, no `pkg-config` and no root access
(DECISIONS.md §1). Consequences that shape the plan:

- Only pure-Rust dependencies can be compiled and tested locally. Crates needing a C/C++
  toolchain (`rquickjs`, `deno_core`, `tree-sitter`, `mlua` vendored, `wasmtime`'s C helpers) could not be
  measured, only assessed from documentation.
- Windows builds, packaging (MSI, Flatpak, AppImage, .deb, .rpm) and GitHub Actions runs cannot be
  executed here. Their configuration is written but unverified until run in CI.

## What is left, in suggested order

1. Done: lazy metadata decode from the mapped index (idle RAM 34 MB, see DECISIONS §9).
2. Done in part: quote, callout, table, note-embed and PNG image widgets. Left: other image formats, math, Mermaid, measured table columns.
3. Done: folding, table editing and vim keys wired into the editor.
3b. Graph view: done on the CPU rasterizer (30 fps at 50k nodes; `wgpu` still needed for 60). Canvas view on top of the existing model: not started.
4. `ext-api`, then move built-in features behind it; scripting; `plugin-host-wasm`.
5. `mermaid-core`, math rendering, syntax highlighting, PDF.
6. `compat-js` and the compatibility harness.
7. Windows build and test, packaging, running the CI workflow for the first time.
