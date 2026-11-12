# Plugin SDK (WASM)

A plugin is a folder `.obsidian/plugins/<id>/` with Obsidian's `manifest.json` and a `main.wasm`.
Enable it by listing the id in `.obsidian/community-plugins.json`; run `plugins:reload` to apply.
Optional manifest section `"walnut": {"permissions": [...]}` with any of `vault.read`,
`vault.write`, `editor.read`, `editor.write`, `ui`. Without it a WASM plugin gets
`vault.read`, `editor.read` and `ui`. Commands, settings (`data.json`) and events need no permission.

Guest exports: `memory`, `alloc(len) -> ptr`, optional `on_load()`, `on_event(ptr, len)` (event JSON),
`run_command(ptr, len) -> i32` (local command id; non-zero = failure), `on_unload()`.
Host import `env.host_call(req_ptr, req_len, resp_ptr, resp_cap) -> i32` takes
`{"method": "...", "params": {...}}` and writes `{"ok": ...}` or `{"error": "..."}`; it returns the
response length, and if that exceeds `resp_cap` nothing is written and the guest calls
`host_call(0, 0, ptr, len)` to fetch it. `examples/plugins/hello-wasm` is a complete no_std example.

Methods (API v1): `api.version`, `log`, `events.subscribe {event}`, `commands.register {id,name,hotkey?}`,
`commands.unregister`, `commands.run {id}`, `vault.list {folder?,ext?}`, `vault.read/exists {path}`,
`vault.write {path,text}`, `vault.delete {path}`, `vault.rename {from,to}`,
`meta.note/links/tags/headings/frontmatter/backlinks {path}`, `meta.resolve {target,source}`,
`editor.active_file/text/selection`, `editor.replace {from,to,text}` (UTF-8 byte offsets),
`ui.notice {text}`, `ui.status {id,text}`, `ui.ribbon {id,icon,title,command}`, `ui.remove {id}`,
`settings.load`, `settings.save {data}`.
Paths are vault-relative; `..`, absolute paths, dot-folders and symlinks leaving the vault are rejected.
Per call limits: 100M fuel, 500 ms wall clock (checked at host calls), 32 MB memory, 10,000 host calls.
