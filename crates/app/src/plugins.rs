//! Extension hosting for the app: discovery, lazy loading, event delivery and the [`Host`]
//! implementation that gives plugins vault and editor access through `ext-api`.

use crate::shell::{App, Key};
use editor_core::{Change, EditKind, Editor, Sel};
use ext_api::{
    Discovered, Event, Extension, Extensions, Host, NoteInfo, PluginInfo, PluginKind, PluginStatus, Selection,
};
use md_engine::frontmatter::Value as Fm;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::path::Path;
use vault_core::{fsio, FileKind};

/// Rounds of event delivery per pump; events raised by plugins while handling events are
/// delivered in the next round, so a plugin that modifies a note on every modify cannot spin.
const MAX_ROUNDS: usize = 4;

#[derive(Default)]
pub struct PluginState {
    pub ext: Extensions,
    pub statuses: Vec<PluginStatus>,
    pub loaded: bool,
    frames: u32,
    events: VecDeque<Event>,
    commands: Vec<String>,
}

impl PluginState {
    pub fn push(&mut self, e: Event) {
        if self.loaded && self.events.len() < 1024 {
            self.events.push_back(e);
        }
    }
}

fn fm_json(v: &Fm) -> Value {
    match v {
        Fm::Null => Value::Null,
        Fm::Bool(b) => json!(b),
        Fm::Number(n) => json!(n),
        Fm::String(s) => json!(s),
        Fm::List(l) => Value::Array(l.iter().map(fm_json).collect()),
        Fm::Map(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), fm_json(v))).collect()),
    }
}

struct AppHost<'a>(&'a mut App);

impl AppHost<'_> {
    fn reload_open_tab(&mut self, rel: &str) {
        let app = &mut *self.0;
        let Some(i) = app.tabs.iter().position(|t| t.path.eq_ignore_ascii_case(rel)) else { return };
        if app.tabs[i].editor.buf.is_modified() {
            return;
        }
        if let Ok((text, stamp)) = app.store.read(rel) {
            let tab = &mut app.tabs[i];
            let head = tab.editor.buf.sels[0].head.min(text.len());
            tab.editor = Editor::new(&text);
            tab.editor.buf.set_sels(vec![Sel::cursor(head)]);
            tab.stamp = stamp;
        }
    }

    fn dirty_tab(&self, rel: &str) -> bool {
        self.0.tabs.iter().any(|t| t.path.eq_ignore_ascii_case(rel) && t.editor.buf.is_modified())
    }
}

impl Host for AppHost<'_> {
    fn vault_root(&self) -> &Path {
        &self.0.store.root
    }

    fn list_files(&mut self) -> Vec<String> {
        self.0.store.index.iter().map(|(_, f)| f.path.to_string()).collect()
    }

    fn read_note(&mut self, rel: &str) -> Result<String, String> {
        if let Some(t) = self.0.tabs.iter().find(|t| t.path.eq_ignore_ascii_case(rel)) {
            if t.editor.buf.is_modified() {
                return Ok(t.editor.buf.text());
            }
        }
        self.0.store.read(rel).map(|(t, _)| t).map_err(|e| e.to_string())
    }

    fn write_note(&mut self, rel: &str, text: &str) -> Result<(), String> {
        if self.dirty_tab(rel) {
            return Err(format!("{rel} has unsaved edits in the editor"));
        }
        let existed = self.0.store.index.id(rel).is_some();
        if existed {
            self.0.store.write(rel, text, None).map_err(|e| e.to_string())?;
        } else {
            self.0.store.create(rel, text).map_err(|e| e.to_string())?;
        }
        self.reload_open_tab(rel);
        self.0.files_changed(&[]);
        self.0.plugins.push(if existed {
            Event::Modify { path: rel.into() }
        } else {
            Event::Create { path: rel.into() }
        });
        Ok(())
    }

    fn delete_note(&mut self, rel: &str) -> Result<(), String> {
        if self.0.store.index.id(rel).is_none() {
            return Err(format!("not found: {rel}"));
        }
        self.0.store.delete(rel).map_err(|e| e.to_string())?;
        self.0.tabs.retain(|t| !t.path.eq_ignore_ascii_case(rel));
        self.0.active = self.0.active.min(self.0.tabs.len().saturating_sub(1));
        self.0.files_changed(&[]);
        self.0.plugins.push(Event::Delete { path: rel.into() });
        Ok(())
    }

    fn rename_note(&mut self, from: &str, to: &str) -> Result<(), String> {
        if self.dirty_tab(from) {
            return Err(format!("{from} has unsaved edits in the editor"));
        }
        let plan = self.0.store.rename(from, to).map_err(|e| e.to_string())?;
        for (old, new) in &plan.moves {
            for t in self.0.tabs.iter_mut().filter(|t| t.path.eq_ignore_ascii_case(old)) {
                t.path = new.clone();
            }
            self.0.plugins.push(Event::Rename { path: new.clone(), old_path: old.clone() });
        }
        for e in &plan.edits {
            self.reload_open_tab(&e.path);
        }
        self.0.files_changed(&[]);
        Ok(())
    }

    fn note_info(&mut self, rel: &str) -> Option<NoteInfo> {
        let entry = self.0.store.index.get(rel)?;
        if entry.kind != FileKind::Markdown {
            return None;
        }
        let meta = entry.meta()?;
        let source = entry.path.to_string();
        let links = meta
            .links
            .iter()
            .map(|l| {
                let resolved = self
                    .0
                    .store
                    .index
                    .resolve(&l.target, &source)
                    .and_then(|id| self.0.store.index.entry(id));
                json!({
                    "target": &*l.target,
                    "subpath": &*l.subpath,
                    "display": l.display_text(),
                    "embed": l.embed,
                    "resolved": resolved.map(|f| f.path.to_string()),
                })
            })
            .collect();
        Some(NoteInfo {
            path: source,
            links,
            tags: meta.all_tags().into_iter().map(str::to_string).collect(),
            headings: meta.headings.iter().map(|h| json!({"level": h.level, "text": &*h.text})).collect(),
            frontmatter: meta.frontmatter.as_ref().map_or(Value::Null, |p| fm_json(&Fm::Map(p.clone()))),
        })
    }

    fn backlinks(&mut self, rel: &str) -> Vec<String> {
        let Some(id) = self.0.store.index.id(rel) else { return Vec::new() };
        let graph = self.0.store.index.graph();
        graph
            .back
            .get(id as usize)
            .into_iter()
            .flatten()
            .filter_map(|b| self.0.store.index.entry(*b).map(|f| f.path.to_string()))
            .collect()
    }

    fn resolve_link(&mut self, target: &str, source: &str) -> Option<String> {
        let id = self.0.store.index.resolve(target, source)?;
        self.0.store.index.entry(id).map(|f| f.path.to_string())
    }

    fn active_file(&mut self) -> Option<String> {
        if self.0.show_graph {
            return None;
        }
        self.0.tab().map(|t| t.path.clone())
    }

    fn editor_text(&mut self) -> Option<String> {
        self.0.tab().map(|t| t.editor.buf.text())
    }

    fn selection(&mut self) -> Option<Selection> {
        let t = self.0.tab()?;
        let s = t.editor.buf.sels[0];
        Some(Selection {
            from: s.start(),
            to: s.end(),
            text: t.editor.buf.slice(s.start(), s.end()).into_owned(),
        })
    }

    fn replace_range(&mut self, from: usize, to: usize, text: &str) -> Result<(), String> {
        let t = self.0.tab_mut().ok_or("no active note")?;
        let buf = &mut t.editor.buf;
        let full = buf.text();
        if from > to || to > full.len() || !full.is_char_boundary(from) || !full.is_char_boundary(to) {
            return Err("range out of bounds".into());
        }
        let now = fsio::now_ms();
        buf.apply(vec![Change { start: from, end: to, text: text.to_string() }], None, EditKind::Other, now);
        t.last_edit_ms = now;
        Ok(())
    }

    fn run_command(&mut self, id: &str) -> Result<(), String> {
        if self.0.plugins.commands.len() >= 64 {
            return Err("too many queued commands".into());
        }
        self.0.plugins.commands.push(id.to_string());
        Ok(())
    }
}

/// Create the runtime for a discovered plugin. Each runtime lives in its own crate behind a
/// cargo feature; a kind that is not compiled in reports that as the plugin's error.
fn instantiate(d: &Discovered, root: &Path) -> Result<Box<dyn Extension>, String> {
    match d.kind {
        #[cfg(feature = "wasm-plugins")]
        PluginKind::Wasm => plugin_host_wasm::load_file(plugin_info(d), &root.join(&d.entry))
            .map(|p| Box::new(p) as Box<dyn Extension>),
        kind => {
            let _ = root;
            Err(format!("no runtime for {kind:?} plugins in this build ({})", d.entry))
        }
    }
}

pub fn plugin_info(d: &Discovered) -> PluginInfo {
    PluginInfo {
        id: d.manifest.id.clone(),
        name: d.manifest.name.clone(),
        perms: d.manifest.permissions(d.kind),
        data_path: d.data_path.clone(),
    }
}

fn status_of(d: &Discovered, enabled: bool) -> PluginStatus {
    PluginStatus {
        id: d.manifest.id.clone(),
        name: d.manifest.name.clone(),
        version: d.manifest.version.clone(),
        kind: d.kind,
        enabled,
        loaded: false,
        error: d.error.clone(),
    }
}

impl App {
    /// Every plugin and script found in the vault with its enabled state, as the settings UI
    /// needs it. Before the plugins have been loaded this scans the disk.
    pub fn plugin_list(&self) -> Vec<PluginStatus> {
        if self.plugins.loaded {
            return self.plugins.statuses.clone();
        }
        let enabled = self.config.enabled_community_plugins();
        ext_api::discover(&self.store.root)
            .iter()
            .map(|d| status_of(d, enabled.contains(&d.manifest.id.as_str())))
            .collect()
    }

    /// Enable or disable a plugin in `.obsidian/community-plugins.json`. Takes effect on the
    /// next `plugins:reload`.
    pub fn set_plugin_enabled(&mut self, id: &str, enabled: bool) -> std::io::Result<()> {
        let mut list: Vec<String> =
            self.config.enabled_community_plugins().into_iter().map(str::to_string).collect();
        list.retain(|p| p != id);
        if enabled {
            list.push(id.to_string());
        }
        self.config.community_plugins.value = json!(list);
        self.config.community_plugins.save().map(|_| ())
    }

    /// Load every enabled plugin (replacing what is loaded). Called after the first frame.
    pub fn plugins_load(&mut self) {
        let mut ext = std::mem::take(&mut self.plugins.ext);
        ext.unload_all(&mut AppHost(self));
        let old: Vec<String> = self.plugins.ext.reg.commands.iter().map(|c| c.id.clone()).collect();
        self.keymap.retain(|(_, id)| !old.contains(id));
        let enabled: Vec<String> =
            self.config.enabled_community_plugins().into_iter().map(str::to_string).collect();
        let root = self.store.root.clone();
        let mut statuses = Vec::new();
        for d in ext_api::discover(&root) {
            let on = enabled.contains(&d.manifest.id);
            let mut st = status_of(&d, on);
            if on && d.error.is_none() {
                match instantiate(&d, &root).and_then(|e| ext.add(&mut AppHost(self), plugin_info(&d), e)) {
                    Ok(()) => st.loaded = true,
                    Err(e) => st.error = Some(e),
                }
            }
            if let Some(e) = &st.error {
                if on {
                    self.notice = Some(format!("Plugin {}: {e}", st.id));
                }
            }
            statuses.push(st);
        }
        self.plugins.ext = ext;
        self.plugins.statuses = statuses;
        self.plugins.loaded = true;
        self.sync_plugin_hotkeys();
        self.plugins.events.clear();
        self.plugins.push(Event::LayoutReady);
        if let Some(t) = self.tab().map(|t| t.path.clone()) {
            self.plugins.push(Event::FileOpen { path: t });
        }
    }

    /// Load an extension that was built in-process (used by tests and embedders).
    pub fn add_extension(&mut self, info: PluginInfo, ext: Box<dyn Extension>) -> Result<(), String> {
        let mut mgr = std::mem::take(&mut self.plugins.ext);
        let r = mgr.add(&mut AppHost(self), info, ext);
        self.plugins.ext = mgr;
        self.plugins.loaded = true;
        self.sync_plugin_hotkeys();
        r
    }

    fn sync_plugin_hotkeys(&mut self) {
        let mut add = Vec::new();
        for c in &self.plugins.ext.reg.commands {
            if let Some(seq) = Key::parse_sequence(&c.hotkey).filter(|s| !s.is_empty()) {
                add.push((seq, c.id.clone()));
            }
        }
        self.keymap.extend(add);
    }

    /// Called from every render. Loads plugins on the second frame (so they never delay the
    /// first), then delivers queued events and commands and collects notices.
    pub fn pump_plugins(&mut self) {
        if !self.plugins.loaded {
            self.plugins.frames += 1;
            if self.plugins.frames < 2 {
                return;
            }
            self.plugins_load();
        }
        for _ in 0..MAX_ROUNDS {
            let events: Vec<Event> = self.plugins.events.drain(..).collect();
            let commands: Vec<String> = std::mem::take(&mut self.plugins.commands);
            if events.is_empty() && commands.is_empty() {
                break;
            }
            let mut ext = std::mem::take(&mut self.plugins.ext);
            for e in &events {
                ext.emit(&mut AppHost(self), e);
            }
            self.plugins.ext = ext;
            for c in commands {
                self.run(&c);
            }
        }
        let notices = self.plugins.ext.reg.take_notices();
        if !notices.is_empty() {
            self.notice = Some(notices.iter().map(|n| n.text.as_str()).collect::<Vec<_>>().join(" | "));
        }
    }

    /// Commands owned by this module: `plugins:list`, `plugins:reload` and plugin-registered ones.
    pub fn run_plugin_command(&mut self, command: &str) -> bool {
        match command {
            "plugins:reload" => {
                self.plugins_load();
                let n = self.plugins.statuses.iter().filter(|s| s.loaded).count();
                self.notice = Some(format!("Reloaded plugins: {n} running"));
                true
            }
            "plugins:list" => {
                let list = self.plugin_list();
                self.notice = Some(if list.is_empty() {
                    "No plugins or scripts found".to_string()
                } else {
                    list.iter()
                        .map(|p| {
                            let state = match (&p.error, p.loaded, p.enabled) {
                                (Some(e), _, true) => format!("error: {e}"),
                                (_, true, _) => "running".into(),
                                (_, _, true) => "enabled".into(),
                                _ => "disabled".into(),
                            };
                            format!("{} ({state})", p.id)
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                });
                true
            }
            _ if self.plugins.ext.reg.command(command).is_some() => {
                let mut ext = std::mem::take(&mut self.plugins.ext);
                ext.run_command(&mut AppHost(self), command);
                self.plugins.ext = ext;
                true
            }
            _ => false,
        }
    }

    /// Palette entries contributed by plugins: (id, name, hotkey).
    pub fn plugin_palette(&self) -> Vec<(String, String)> {
        self.plugins.ext.reg.commands.iter().map(|c| (c.id.clone(), c.name.clone())).collect()
    }
}
