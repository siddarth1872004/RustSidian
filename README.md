# Rustwalnut

An Obsidian-vault-compatible knowledge base written in Rust: no Electron, no webview, no
JavaScript in the core. It opens an existing Obsidian vault without migration and keeps its own
extra data in `.walnut/`.

**Status: partial.** The headless core (Markdown, vault index, search, graph model, editor model,
CSS engine, JSON Canvas) and a working desktop shell are implemented and tested. Large parts of
the original specification are **not implemented**: plugins (WASM and JS), Mermaid, Bases,
scripting, math and PDF rendering, the canvas view, packaging. The checklist below is
the authoritative list; nothing outside it is claimed.

## Build and run

```sh
cargo build --release -p walnut          # 6.5 MB binary at target/release/walnut
target/release/walnut examples/vault     # open the demo vault in a window
target/release/walnut help               # CLI: index, search, export, doctor, screenshot, commands
cargo test --workspace                   # 168 tests
cargo run --release -p walnut --example perf_budget    # performance gate (headless)
scripts/window-budget.sh <vault dir>     # performance gate (needs a display)
```

Requires stable Rust 1.85 or later. All dependencies are pure Rust; no C toolchain is needed.
Verified on Linux (WSL2 Ubuntu under WSLg, Wayland and X11 backends compiled in). Windows
(`x86_64-pc-windows-gnu`) and macOS (`aarch64-apple-darwin`) pass `cargo check --all-targets` and
Windows also passes clippy, but **neither has been built, linked or run**.

## Install

Release archives (Linux x86_64, Windows x86_64, macOS arm64 and x86_64, with SHA256SUMS.txt) are
built by `.github/workflows/release.yml` when a `v*` tag is pushed and attached to the GitHub
Release. No release has been published yet. On Linux, `packaging/walnut.desktop` and
`packaging/walnut.svg` can be copied to `~/.local/share/applications` and
`~/.local/share/icons/hicolor/scalable/apps`.

## Build from source

```sh
git clone https://github.com/siddarth1872004/RustSidian && cd RustSidian
cargo build --release -p walnut      # target/release/walnut (walnut.exe on Windows)
```

Linux needs only a normal system linker (`build-essential`); Windows needs the MSVC build tools.
The repo forces no linker settings. A machine with no C toolchain can link with `rust-lld` through
a git-ignored `.cargo/config.toml` of its own. Licensed under MIT OR Apache-2.0
(`LICENSE-MIT`, `LICENSE-APACHE`).

## Measured against the budgets

Machine: WSL2 Ubuntu, 12 logical cores, 7 GB RAM, software rendering (no GPU passthrough).
Synthetic vaults from `perf_budget`.

| Budget | Target | Measured | |
|---|---|---|---|
| Release binary, stripped | < 15 MB (aim < 10) | 6.5 MB | met |
| Cold start to presented frame, 5,000 notes | < 150 ms | 65–98 ms (worst of 5 runs, three sessions) | met |
| Cold start to presented frame, 50,000 notes | < 400 ms | 188–260 ms | met |
| Idle RAM, 10,000 notes | < 80 MB | 34 MB | met |
| Idle CPU | 0% | 0 CPU ticks over 5 s | met |
| Keystroke to pixels, 1 MB note | < 8 ms | 4.9 ms median, 6.2 ms p95, 8.7 ms worst seen | met at p95 |
| Scroll frame, 5 MB note | display rate | 7.4 ms mean (60 Hz needs 16.7 ms; 144 Hz would need 6.9 ms) | met for 60 Hz |
| Full-text search, 50,000 notes | first results < 30 ms | 8–31 ms depending on query; `line:` queries sit at 29–31 ms | met, except `line:` which is on the line |
| Quick switcher, 50,000 paths | < 5 ms per keystroke | 1.8–2.0 ms | met |
| Graph, 50,000 nodes at 60 fps | pan/zoom | 33 ms per frame (30 fps) at 50,000 nodes / 600,000 links, 29 ms at 10,000 / 120,000, 1.7 ms for a small vault; CPU rasterizer, links sampled past 8M line pixels; layout tick 26 ms on a worker thread | **missed** |
| Mermaid render | < 50 ms | not implemented | n/a |

Caveats, stated plainly:

- "Cold start" means a fresh process with the persisted index present. The very first open of a
  vault has to parse every note: 47 ms at 5,000 notes, 433 ms at 50,000.
- Start-up and typing figures are for the software renderer and include everything up to the
  frame being handed to the compositor, not the compositor's own latency.
- Idle RAM counts resident pages after the mapped index has been released (`madvise`). Note
  metadata is decoded from the mapped index on demand, so a vault-wide metadata query (graph
  build, `tag:` search) briefly raises it. On Windows there is no release call yet.
- The budgets are gated by `perf_budget` and `scripts/window-budget.sh`. They are wired into
  `.github/workflows/ci.yml` and `release.yml`, which **have never been run**.

## Feature checklist

`done` = implemented and covered by tests. `partial` = usable, gaps listed. `none` = not implemented.

### Vault and files

| Feature | Status | Notes |
|---|---|---|
| Open an Obsidian vault with zero migration | done | Dot-folders are skipped like Obsidian does |
| Read/write `.obsidian/*.json`, unknown keys preserved | done | `app`, `appearance`, `core-plugins`, `community-plugins`, `hotkeys`, `bookmarks`, `workspace`, `graph`. Only `app`, `appearance`, `hotkeys` and `graph` are *acted on*; the rest are loaded and preserved but unused |
| Atomic writes | done | temp file + fsync + rename |
| Conflict detection with external edits | done | A changed file is never overwritten |
| File watcher with debounce | done | Linux inotify verified; other platforms untested |
| Metadata cache (links, embeds, tags, headings, blocks, front matter, list items, sections) | done | |
| Link resolution (Obsidian rules) | done | |
| Rename with link updates | done | Property-tested over 40,000 random vaults. Links in `.canvas` files are **not** updated by the app yet (`canvas-core` has the function) |
| Persisted index, mmap, mtime/size revalidation, rebuild on corruption | done | |
| Trash | partial | Vault `.trash/` and XDG trash; Windows recycle bin falls back to `.trash/` |
| File recovery snapshots | partial | Snapshots are written on save; there is no UI to browse or restore them |

### Markdown

| Feature | Status | Notes |
|---|---|---|
| CommonMark + GFM blocks and inlines | partial | Not run against the CommonMark spec suite; known simplifications in lazy continuation and HTML blocks |
| Wikilinks, embeds, block ids, callouts, comments, highlights, tags, footnotes, front matter, task lists, tables, math spans | done | Parsed with byte-exact spans |
| Incremental re-parse | done | Property-tested equal to a full parse (60,000 cases) |
| HTML rendering | done | Used by export |
| Math rendering | none | Parsed, shown as source |
| Code syntax highlighting | none | |
| Fuzzing with `cargo-fuzz` | none | Property tests with arbitrary input stand in for it |

### Editor

| Feature | Status | Notes |
|---|---|---|
| Source mode, live preview | partial | Inline markup is hidden/revealed around the cursor. Quotes, callouts (coloured by type, foldable ones collapsed when written `-`) and tables are rendered while the cursor is outside them and turn back into source when it enters. Table columns are sized by estimate, not measurement; right alignment is not applied; callouts have no icons and no fold toggle. Math and Mermaid are shown as source |
| Reading mode | partial | Live preview without a cursor; not a separate rendered document |
| Multi-cursor, undo tree, smart lists, auto-pair, indent, formatting toggles, move/duplicate line | done | |
| Folding | done | Headings, list items, quotes/callouts and code blocks. Arrow next to each heading, `Toggle fold`, `Fold all`, `Unfold all`. Folds follow edits and undo; they are not saved between sessions. While any fold is active the whole note is rescanned after each edit |
| Table editing | partial | In live preview with the cursor in a table: Tab/Shift+Tab/Enter move between cells and re-align the source; palette commands add, move, delete, align and sort rows and columns, and insert a table. Editing happens in the table's source, not in the rendered grid |
| Vim mode | partial | On when `vimMode` is set in `.obsidian/app.json`, or with `Toggle Vim key bindings`. Subset documented in `editor-core/src/vim.rs`; the mode is shown in the status bar; the caret does not change shape |
| IME | partial | Committed text is accepted; no pre-edit display. fcitx/ibus untested |
| RTL, ligatures | partial | Whatever `cosmic-text` shaping provides; not tested |
| Spellcheck, drag and drop, paste of files/images, properties editor, hover preview, slash commands | none | |
| Embeds of notes and images | partial | `![[Note]]`, `![[Note#Heading]]`, `![[Note#^block]]` show the note or section below the paragraph (first 30 blocks; tables and nested embeds inside it as source). `![[image.png\|100x80]]` and `![](image.png)` show PNG images only, decoded on the UI thread at first display; JPEG, GIF, WebP, SVG and remote images are not shown |
| Embeds of audio, video, PDF | none | |

### Navigation and views

| Feature | Status | Notes |
|---|---|---|
| File explorer | partial | Virtualised, collapsible; no sorting options, drag-rearrange, rename or context menu |
| Quick switcher, command palette | done | |
| Search panel with Obsidian operators | done | All operators; no embedded `query` blocks |
| Back/forward, tabs | done | No split panes, stacked or pinned tabs, pop-out windows, linked panes, workspaces |
| Outline, backlinks (linked), word count | done | No unlinked mentions, outgoing-links or tags panes |
| Daily notes, random note | partial | Daily note uses the UTC date and `YYYY-MM-DD`; custom formats and templates are not supported |
| Graph view | partial | Global and local graph (depth 1-3) in the main pane: pan, zoom about the pointer, drag nodes, hover highlight, click to open (unresolved creates the note, tags search). Filters and arrows as header chips, read from and saved to `graph.json`; search filter and color groups from `graph.json` are applied but have no UI. No force sliders, no layout cache on disk, dense graphs draw a sample of links |
| Canvas view | done | Text/file/link/group nodes, colors, curved edges with arrows and labels; pan, wheel zoom, move, resize, double-click to edit text, add/delete node, lossless save. No edge editing; file nodes show a text preview |
| Bookmarks, tags pane, properties view, periodic notes, templates, unique note creator, slides, format converter, audio recorder, footnotes view | none | |
| Bases | none | |

### Theming and customisation

| Feature | Status | Notes |
|---|---|---|
| CSS subset engine | done | See `THEME_COMPAT.md` for the exact subset |
| Themes from `.obsidian/themes/`, snippets from `.obsidian/snippets/`, live reload | partial | Loaded and reloaded on change. **No community theme has been tested**; themes that rely on unsupported CSS will apply only in part |
| Light/dark | partial | Follows `appearance.json` and a toggle command; does not follow the OS setting |
| Commands with stable ids, rebindable through `hotkeys.json`, key sequences | done | No visual hotkey editor |
| Everything else in the customisation section (TOML config, JSON Schema, data-driven layout, theme inspector UI, scripting, feature flags, profiles, portable mode, `--config-dir`) | none | The theme engine has an `inspect` API; no UI uses it |

### Export, plugins, platform

| Feature | Status | Notes |
|---|---|---|
| Export HTML, Markdown bundle | done | |
| Export PDF, print | none | |
| Native WASM plugins, plugin SDK, `walnut plugin` | none | |
| Obsidian JS plugin compatibility, CM6 adapter, community browser | none | Only the JS engine spike was done |
| Accessibility (`accesskit`), high contrast, reduced motion | none | Keyboard operation works for what exists; there is no screen-reader support |
| GPU rendering (`wgpu`) | none | Software rendering only, by measured decision (DECISIONS.md §2) |
| Multi-window, system tray, file dialogs, URI scheme, file associations, single-instance IPC | none | |
| Packaging (MSI, zip, Flatpak, AppImage, deb, rpm) | none | Release workflow produces tar.gz/zip archives (unrun); `.desktop` file and SVG icon included |
| `walnut doctor`, `--profile`, perf overlay (Ctrl+Shift+F12) | done | |

## Layout

```
crates/md-engine     Markdown parser, incremental re-parse, HTML, front matter
crates/vault-core    index, link resolution, rename, cache, IO, watcher, .obsidian config
crates/search-core   query language, corpus scan, fuzzy matcher
crates/graph-core    graph model, filters, Barnes-Hut layout, graph.json
crates/editor-core   rope buffer, undo tree, commands, folding, decorations, tables, vim
crates/theme-engine  CSS parser, selectors, cascade, values
crates/canvas-core   JSON Canvas model
crates/walnut-ui     element tree, flex layout, software renderer
crates/app           shell, commands, view, CLI, window  (binary: walnut)
spikes/              the three risk spikes behind DECISIONS.md
examples/vault/      demo vault
```

`PLAN.md` has the per-crate ledger, `DECISIONS.md` the tradeoffs with measurements.

## Deliverables from the specification that were not produced

Packaged releases; `criterion` benchmarks (the gates are plain timing programs instead); `insta`
snapshots beyond `md-engine`; visual-regression screenshots; `cargo-fuzz` targets; the
vault-compatibility suite on real-world vaults; sample WASM plugins, sample JS plugins, sample
themes and macros. `examples/vault/` has one sample CSS snippet. `cargo deny` has a config file
but was never run.
