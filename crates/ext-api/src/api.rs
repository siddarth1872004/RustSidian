use crate::registry::{CommandDef, RibbonItem, StatusItem, MAX_ITEMS_PER_PLUGIN};
use crate::{guard_path, Permission, PluginInfo, Registry};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const API_VERSION: u32 = 1;
const MAX_TEXT: usize = 16 << 20;
const MAX_SETTINGS: usize = 4 << 20;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NoteInfo {
    pub path: String,
    pub links: Vec<Value>,
    pub tags: Vec<String>,
    pub headings: Vec<Value>,
    pub frontmatter: Value,
}

/// A selection in the active note, in UTF-8 byte offsets.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Selection {
    pub from: usize,
    pub to: usize,
    pub text: String,
}

/// What the app provides to extensions. All paths are vault-relative and already validated.
pub trait Host {
    fn vault_root(&self) -> &Path;
    fn list_files(&mut self) -> Vec<String>;
    fn read_note(&mut self, path: &str) -> Result<String, String>;
    /// Create the note or overwrite it.
    fn write_note(&mut self, path: &str, text: &str) -> Result<(), String>;
    fn delete_note(&mut self, path: &str) -> Result<(), String>;
    /// Rename or move, updating links that point at it.
    fn rename_note(&mut self, from: &str, to: &str) -> Result<(), String>;
    fn note_info(&mut self, path: &str) -> Option<NoteInfo>;
    fn backlinks(&mut self, path: &str) -> Vec<String>;
    fn resolve_link(&mut self, target: &str, source: &str) -> Option<String>;
    fn active_file(&mut self) -> Option<String>;
    fn editor_text(&mut self) -> Option<String>;
    fn selection(&mut self) -> Option<Selection>;
    fn replace_range(&mut self, from: usize, to: usize, text: &str) -> Result<(), String>;
    /// Run a built-in or plugin command; the app may defer it until the current call returns.
    fn run_command(&mut self, id: &str) -> Result<(), String>;
}

/// One call context: the registry plus the host, handed to extensions while they run.
pub struct Api<'a> {
    pub reg: &'a mut Registry,
    pub host: &'a mut dyn Host,
}

fn str_param<'v>(p: &'v Value, key: &str) -> Result<&'v str, String> {
    p.get(key).and_then(Value::as_str).ok_or_else(|| format!("missing string parameter {key:?}"))
}

fn usize_param(p: &Value, key: &str) -> Result<usize, String> {
    p.get(key)
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .ok_or_else(|| format!("missing integer parameter {key:?}"))
}

fn need(plugin: &PluginInfo, perm: Permission, name: &str) -> Result<(), String> {
    if plugin.perms.has(perm) {
        Ok(())
    } else {
        Err(format!("permission denied: {name}"))
    }
}

fn valid_local_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

impl Api<'_> {
    /// The JSON call boundary: `{"method","params"}` in, `{"ok"}` or `{"error"}` out.
    pub fn call(&mut self, plugin: &PluginInfo, request: &str) -> String {
        let out = match serde_json::from_str::<Value>(request) {
            Ok(req) => {
                let params = req.get("params").cloned().unwrap_or(Value::Null);
                match req.get("method").and_then(Value::as_str) {
                    Some(m) => self.dispatch(plugin, m, &params),
                    None => Err("missing method".to_string()),
                }
            }
            Err(e) => Err(format!("bad request: {e}")),
        };
        match out {
            Ok(v) => json!({ "ok": v }),
            Err(e) => json!({ "error": e }),
        }
        .to_string()
    }

    fn path(&self, p: &Value, key: &str) -> Result<String, String> {
        guard_path(self.host.vault_root(), str_param(p, key)?)
    }

    pub fn dispatch(&mut self, plugin: &PluginInfo, method: &str, p: &Value) -> Result<Value, String> {
        use Permission::*;
        match method {
            "api.version" => Ok(json!(API_VERSION)),
            "log" => {
                self.reg.push_log(&plugin.id, str_param(p, "text")?.chars().take(2000).collect());
                Ok(Value::Null)
            }
            "events.subscribe" => {
                self.reg.subscribe(&plugin.id, str_param(p, "event")?);
                Ok(Value::Null)
            }
            "commands.register" => {
                let local = str_param(p, "id")?;
                if !valid_local_id(local) {
                    return Err(format!("invalid command id {local:?}"));
                }
                let id = format!("{}:{local}", plugin.id);
                let name = format!("{}: {}", plugin.name, str_param(p, "name")?);
                let hotkey = p.get("hotkey").and_then(Value::as_str).unwrap_or("").to_string();
                self.reg.commands.retain(|c| c.id != id);
                if self.reg.commands.iter().filter(|c| c.plugin == plugin.id).count() >= MAX_ITEMS_PER_PLUGIN
                {
                    return Err("too many commands".into());
                }
                self.reg.commands.push(CommandDef {
                    id: id.clone(),
                    name,
                    plugin: plugin.id.clone(),
                    hotkey,
                });
                Ok(json!(id))
            }
            "commands.unregister" => {
                let id = format!("{}:{}", plugin.id, str_param(p, "id")?);
                self.reg.commands.retain(|c| c.id != id);
                Ok(Value::Null)
            }
            "commands.run" => {
                need(plugin, Ui, "ui")?;
                self.host.run_command(str_param(p, "id")?).map(|_| Value::Null)
            }
            "vault.list" => {
                need(plugin, VaultRead, "vault.read")?;
                let folder =
                    p.get("folder").and_then(Value::as_str).unwrap_or("").trim_matches('/').to_string();
                let ext = p.get("ext").and_then(Value::as_str).map(str::to_lowercase);
                let files: Vec<String> = self
                    .host
                    .list_files()
                    .into_iter()
                    .filter(|f| folder.is_empty() || f.starts_with(&format!("{folder}/")))
                    .filter(|f| ext.as_ref().is_none_or(|e| f.to_lowercase().ends_with(&format!(".{e}"))))
                    .collect();
                Ok(json!(files))
            }
            "vault.read" => {
                need(plugin, VaultRead, "vault.read")?;
                let path = self.path(p, "path")?;
                self.host.read_note(&path).map(Value::String)
            }
            "vault.exists" => {
                need(plugin, VaultRead, "vault.read")?;
                let path = self.path(p, "path")?;
                Ok(json!(self.host.list_files().iter().any(|f| f.eq_ignore_ascii_case(&path))))
            }
            "vault.write" => {
                need(plugin, VaultWrite, "vault.write")?;
                let path = self.path(p, "path")?;
                let text = str_param(p, "text")?;
                if text.len() > MAX_TEXT {
                    return Err("text too large".into());
                }
                self.host.write_note(&path, text).map(|_| Value::Null)
            }
            "vault.delete" => {
                need(plugin, VaultWrite, "vault.write")?;
                let path = self.path(p, "path")?;
                self.host.delete_note(&path).map(|_| Value::Null)
            }
            "vault.rename" => {
                need(plugin, VaultWrite, "vault.write")?;
                let (from, to) = (self.path(p, "from")?, self.path(p, "to")?);
                self.host.rename_note(&from, &to).map(|_| Value::Null)
            }
            "meta.note" | "meta.links" | "meta.tags" | "meta.frontmatter" | "meta.headings" => {
                need(plugin, VaultRead, "vault.read")?;
                let path = self.path(p, "path")?;
                let info = self.host.note_info(&path).ok_or_else(|| format!("no such note: {path}"))?;
                Ok(match method {
                    "meta.links" => json!(info.links),
                    "meta.tags" => json!(info.tags),
                    "meta.headings" => json!(info.headings),
                    "meta.frontmatter" => info.frontmatter,
                    _ => json!(info),
                })
            }
            "meta.backlinks" => {
                need(plugin, VaultRead, "vault.read")?;
                let path = self.path(p, "path")?;
                Ok(json!(self.host.backlinks(&path)))
            }
            "meta.resolve" => {
                need(plugin, VaultRead, "vault.read")?;
                let source = p.get("source").and_then(Value::as_str).unwrap_or("");
                Ok(json!(self.host.resolve_link(str_param(p, "target")?, source)))
            }
            "editor.active_file" => {
                need(plugin, EditorRead, "editor.read")?;
                Ok(json!(self.host.active_file()))
            }
            "editor.text" => {
                need(plugin, EditorRead, "editor.read")?;
                Ok(json!(self.host.editor_text()))
            }
            "editor.selection" => {
                need(plugin, EditorRead, "editor.read")?;
                Ok(json!(self.host.selection()))
            }
            "editor.replace" => {
                need(plugin, EditorWrite, "editor.write")?;
                let text = str_param(p, "text")?;
                if text.len() > MAX_TEXT {
                    return Err("text too large".into());
                }
                self.host
                    .replace_range(usize_param(p, "from")?, usize_param(p, "to")?, text)
                    .map(|_| Value::Null)
            }
            "ui.notice" => {
                need(plugin, Ui, "ui")?;
                self.reg.push_notice(&plugin.id, str_param(p, "text")?.chars().take(500).collect());
                Ok(Value::Null)
            }
            "ui.status" => {
                need(plugin, Ui, "ui")?;
                let id = str_param(p, "id")?.to_string();
                let text: String = str_param(p, "text")?.chars().take(120).collect();
                if let Some(s) = self.reg.status.iter_mut().find(|s| s.plugin == plugin.id && s.id == id) {
                    s.text = text;
                } else if self.reg.status.iter().filter(|s| s.plugin == plugin.id).count()
                    < MAX_ITEMS_PER_PLUGIN
                {
                    self.reg.status.push(StatusItem { plugin: plugin.id.clone(), id, text });
                } else {
                    return Err("too many status items".into());
                }
                Ok(Value::Null)
            }
            "ui.ribbon" => {
                need(plugin, Ui, "ui")?;
                let id = str_param(p, "id")?.to_string();
                let item = RibbonItem {
                    plugin: plugin.id.clone(),
                    icon: p.get("icon").and_then(Value::as_str).unwrap_or("●").chars().take(4).collect(),
                    title: p.get("title").and_then(Value::as_str).unwrap_or("").to_string(),
                    command: format!("{}:{}", plugin.id, str_param(p, "command")?),
                    id,
                };
                self.reg.ribbon.retain(|r| !(r.plugin == plugin.id && r.id == item.id));
                if self.reg.ribbon.iter().filter(|r| r.plugin == plugin.id).count() >= MAX_ITEMS_PER_PLUGIN {
                    return Err("too many ribbon items".into());
                }
                self.reg.ribbon.push(item);
                Ok(Value::Null)
            }
            "ui.remove" => {
                need(plugin, Ui, "ui")?;
                let id = str_param(p, "id")?;
                self.reg.status.retain(|s| !(s.plugin == plugin.id && s.id == id));
                self.reg.ribbon.retain(|r| !(r.plugin == plugin.id && r.id == id));
                Ok(Value::Null)
            }
            "settings.load" => {
                let file = self.settings_file(plugin)?;
                match std::fs::read_to_string(&file) {
                    Ok(t) => {
                        serde_json::from_str(&t).map_err(|e| format!("settings are not valid JSON: {e}"))
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
                    Err(e) => Err(e.to_string()),
                }
            }
            "settings.save" => {
                let file = self.settings_file(plugin)?;
                let text = serde_json::to_string_pretty(p.get("data").unwrap_or(&Value::Null))
                    .map_err(|e| e.to_string())?;
                if text.len() > MAX_SETTINGS {
                    return Err("settings too large".into());
                }
                if let Some(dir) = file.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                }
                let tmp = file.with_extension("json.tmp");
                std::fs::write(&tmp, text)
                    .and_then(|_| std::fs::rename(&tmp, &file))
                    .map_err(|e| e.to_string())?;
                Ok(Value::Null)
            }
            other => Err(format!("unknown method {other:?}")),
        }
    }

    fn settings_file(&self, plugin: &PluginInfo) -> Result<PathBuf, String> {
        let root = self.host.vault_root();
        let rel = plugin.data_path.as_str();
        if rel.contains("..") || rel.starts_with('/') {
            return Err("invalid settings path".into());
        }
        let file = root.join(rel);
        if let (Some(dir), Ok(real_root)) = (file.parent(), root.canonicalize()) {
            let mut probe = dir.to_path_buf();
            loop {
                if let Ok(real) = probe.canonicalize() {
                    if !real.starts_with(&real_root) {
                        return Err("settings path escapes the vault".into());
                    }
                    break;
                }
                if !probe.pop() {
                    break;
                }
            }
        }
        Ok(file)
    }
}
