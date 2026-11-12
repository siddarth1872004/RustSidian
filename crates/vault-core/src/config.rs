//! `.obsidian/*.json` configuration files. Each file is kept as a JSON document so keys this
//! program does not know about survive a load/save cycle unchanged and in their original order.

use crate::fsio::atomic_write;
use crate::vault::LinkFormat;
use serde_json::{Map, Value};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct JsonFile {
    pub path: PathBuf,
    pub value: Value,
    /// The value as loaded (or the default for a missing file); the file is only written when
    /// the current value differs from it.
    original: Value,
}

impl JsonFile {
    /// Load a config file. A missing file yields `default` (an empty object for most files);
    /// a file that is not valid JSON is an error and is never overwritten implicitly.
    pub fn load(path: &Path, default: Value) -> io::Result<JsonFile> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let value: Value = serde_json::from_str(&text).map_err(|e| {
                    io::Error::new(io::ErrorKind::InvalidData, format!("{}: {e}", path.display()))
                })?;
                Ok(JsonFile { path: path.to_path_buf(), original: value.clone(), value })
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                Ok(JsonFile { path: path.to_path_buf(), original: default.clone(), value: default })
            }
            Err(e) => Err(e),
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.value.get(key)
    }
    pub fn str(&self, key: &str) -> Option<&str> {
        self.get(key)?.as_str()
    }
    pub fn bool(&self, key: &str, default: bool) -> bool {
        self.get(key).and_then(Value::as_bool).unwrap_or(default)
    }
    pub fn set(&mut self, key: &str, v: Value) {
        if !self.value.is_object() {
            self.value = Value::Object(Map::new());
        }
        if let Some(obj) = self.value.as_object_mut() {
            obj.insert(key.to_string(), v);
        }
    }
    pub fn remove(&mut self, key: &str) {
        if let Some(obj) = self.value.as_object_mut() {
            obj.shift_remove(key);
        }
    }

    /// Serialized the way Obsidian writes its config: two-space indentation.
    pub fn to_text(&self) -> String {
        serde_json::to_string_pretty(&self.value).unwrap_or_else(|_| "{}".into())
    }

    /// Write the file if its content differs from what was loaded. Returns whether it wrote.
    pub fn save(&mut self) -> io::Result<bool> {
        if self.value == self.original {
            return Ok(false);
        }
        atomic_write(&self.path, self.to_text().as_bytes())?;
        self.original = self.value.clone();
        Ok(true)
    }
}

/// The set of Obsidian config files Rustsidian reads and writes.
pub struct ObsidianConfig {
    pub dir: PathBuf,
    pub app: JsonFile,
    pub appearance: JsonFile,
    /// Either the legacy array of ids or the newer `{id: bool}` object.
    pub core_plugins: JsonFile,
    pub community_plugins: JsonFile,
    pub hotkeys: JsonFile,
    pub bookmarks: JsonFile,
    pub workspace: JsonFile,
    pub graph: JsonFile,
}

impl ObsidianConfig {
    pub fn load(vault_root: &Path) -> io::Result<ObsidianConfig> {
        let dir = vault_root.join(".obsidian");
        let obj = || Value::Object(Map::new());
        let f = |name: &str, default: Value| JsonFile::load(&dir.join(name), default);
        Ok(ObsidianConfig {
            app: f("app.json", obj())?,
            appearance: f("appearance.json", obj())?,
            core_plugins: f("core-plugins.json", Value::Array(Vec::new()))?,
            community_plugins: f("community-plugins.json", Value::Array(Vec::new()))?,
            hotkeys: f("hotkeys.json", obj())?,
            bookmarks: f("bookmarks.json", obj())?,
            workspace: f("workspace.json", obj())?,
            graph: f("graph.json", obj())?,
            dir,
        })
    }

    pub fn save_all(&mut self) -> io::Result<usize> {
        let mut n = 0;
        for f in [
            &mut self.app,
            &mut self.appearance,
            &mut self.core_plugins,
            &mut self.community_plugins,
            &mut self.hotkeys,
            &mut self.bookmarks,
            &mut self.workspace,
            &mut self.graph,
        ] {
            n += f.save()? as usize;
        }
        Ok(n)
    }

    pub fn link_format(&self) -> LinkFormat {
        match self.app.str("newLinkFormat") {
            Some("relative") => LinkFormat::Relative,
            Some("absolute") => LinkFormat::Absolute,
            _ => LinkFormat::Shortest,
        }
    }
    pub fn use_markdown_links(&self) -> bool {
        self.app.bool("useMarkdownLinks", false)
    }
    pub fn attachment_folder(&self) -> &str {
        self.app.str("attachmentFolderPath").unwrap_or("/")
    }
    pub fn trash_option(&self) -> crate::fsio::TrashMode {
        match self.app.str("trashOption") {
            Some("system") => crate::fsio::TrashMode::System,
            Some("none") => crate::fsio::TrashMode::Permanent,
            _ => crate::fsio::TrashMode::Vault,
        }
    }
    pub fn css_theme(&self) -> Option<&str> {
        self.appearance.str("cssTheme").filter(|s| !s.is_empty())
    }
    /// `obsidian` = dark, `moonstone` = light, anything else follows the system.
    pub fn color_scheme(&self) -> Option<bool> {
        match self.appearance.str("theme") {
            Some("obsidian") => Some(true),
            Some("moonstone") => Some(false),
            _ => None,
        }
    }
    pub fn enabled_snippets(&self) -> Vec<&str> {
        self.appearance
            .get("enabledCssSnippets")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }
    pub fn enabled_community_plugins(&self) -> Vec<&str> {
        self.community_plugins
            .value
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }

    /// Core plugin state in either file format. `None` means the file does not mention it.
    pub fn core_plugin_enabled(&self, id: &str) -> Option<bool> {
        match &self.core_plugins.value {
            Value::Array(a) if a.is_empty() => None,
            Value::Array(a) => Some(a.iter().any(|v| v.as_str() == Some(id))),
            Value::Object(o) => o.get(id).and_then(Value::as_bool),
            _ => None,
        }
    }
    pub fn set_core_plugin(&mut self, id: &str, enabled: bool) {
        match &mut self.core_plugins.value {
            Value::Object(o) => {
                o.insert(id.to_string(), Value::Bool(enabled));
            }
            Value::Array(a) => {
                a.retain(|v| v.as_str() != Some(id));
                if enabled {
                    a.push(Value::String(id.to_string()));
                }
            }
            other => *other = Value::Object(Map::from_iter([(id.to_string(), Value::Bool(enabled))])),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn unknown_keys_and_order_survive() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join(".obsidian")).unwrap();
        let app = "{\n  \"zzzFutureKey\": {\n    \"nested\": [\n      1,\n      2\n    ]\n  },\n  \"newLinkFormat\": \"relative\",\n  \"alwaysUpdateLinks\": true\n}";
        fs::write(d.path().join(".obsidian/app.json"), app).unwrap();
        fs::write(d.path().join(".obsidian/core-plugins.json"), "[\"graph\",\"backlink\"]").unwrap();
        let mut c = ObsidianConfig::load(d.path()).unwrap();
        assert_eq!(c.link_format(), LinkFormat::Relative);
        assert_eq!(c.save_all().unwrap(), 0, "loading and saving without changes writes nothing");
        c.app.set("useMarkdownLinks", Value::Bool(true));
        assert_eq!(c.save_all().unwrap(), 1);
        let text = fs::read_to_string(d.path().join(".obsidian/app.json")).unwrap();
        assert_eq!(text, app.replace("true\n}", "true,\n  \"useMarkdownLinks\": true\n}"));
        assert_eq!(c.core_plugin_enabled("graph"), Some(true));
        assert_eq!(c.core_plugin_enabled("canvas"), Some(false));
        c.set_core_plugin("canvas", true);
        c.set_core_plugin("graph", false);
        assert_eq!(c.core_plugins.to_text().replace([' ', '\n'], ""), "[\"backlink\",\"canvas\"]");
        assert!(!d.path().join(".obsidian/graph.json").exists(), "absent files are not created by a save");
    }

    #[test]
    fn object_style_core_plugins_and_invalid_json() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join(".obsidian")).unwrap();
        fs::write(d.path().join(".obsidian/core-plugins.json"), "{\"graph\": true, \"canvas\": false}")
            .unwrap();
        let c = ObsidianConfig::load(d.path()).unwrap();
        assert_eq!(
            (c.core_plugin_enabled("graph"), c.core_plugin_enabled("canvas"), c.core_plugin_enabled("x")),
            (Some(true), Some(false), None)
        );
        fs::write(d.path().join(".obsidian/app.json"), "{ not json").unwrap();
        assert!(ObsidianConfig::load(d.path()).is_err());
        assert_eq!(fs::read_to_string(d.path().join(".obsidian/app.json")).unwrap(), "{ not json");
    }
}
