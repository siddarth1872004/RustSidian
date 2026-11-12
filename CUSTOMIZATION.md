# Customisation reference

This covers what exists today. The specification's broader customisation layer (TOML config,
JSON Schemas, data-driven layout, scripting, feature flags, profiles, portable mode) is **not
implemented**.

## Hotkeys

`.obsidian/hotkeys.json`, Obsidian's format. Each command id maps to a list of bindings; an empty
list unbinds the command. `Mod` means Ctrl.

```json
{
  "editor:toggle-highlight": [{ "modifiers": ["Mod", "Shift"], "key": "H" }],
  "editor:toggle-bold": []
}
```

Default bindings may be key sequences internally (`Ctrl+K Ctrl+S` style); `hotkeys.json` entries
are single chords. Changes are picked up when the file is saved. There is no visual editor.

`walnut commands` prints every command id with its default binding. All of them are also
reachable from the command palette (Ctrl+P).

## Appearance

`.obsidian/appearance.json`:

| Key | Effect |
|---|---|
| `theme` | `obsidian` = dark, `moonstone` = light, absent = dark. The OS setting is not followed. |
| `cssTheme` | Folder name under `.obsidian/themes/` whose `theme.css` is loaded |
| `enabledCssSnippets` | Names of files under `.obsidian/snippets/` (without `.css`) |

What CSS can change is listed in `THEME_COMPAT.md`. Per-note `cssclasses` are parsed into the
metadata cache but not yet applied to the view.

## Behaviour read from `.obsidian/app.json`

| Key | Effect |
|---|---|
| `newLinkFormat` | `shortest` (default), `relative`, `absolute`: how links are rewritten on rename |
| `trashOption` | `local` (default, `.trash/`), `system` (XDG trash on Linux), `none` (delete) |

`useMarkdownLinks` and `attachmentFolderPath` are read but nothing uses them yet.

## Pluggable internals available as Rust APIs only

These exist in the libraries but have no configuration surface: Markdown extensions
(`md_engine::Options`), filename sanitisation (`vault_core::path::SanitizePolicy`), search
ranking weights (`search_core::RankWeights`), graph forces (`graph_core::Forces`), editor
auto-pair/smart-list/tab settings (`editor_core::Settings`), undo merge window.
