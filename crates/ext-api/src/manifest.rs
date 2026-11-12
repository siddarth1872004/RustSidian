use crate::{Permissions, PluginKind};
use serde::Deserialize;
use std::path::Path;

/// `manifest.json` as Obsidian defines it, plus an optional `walnut.permissions` list.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub min_app_version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub author_url: String,
    #[serde(default)]
    pub is_desktop_only: bool,
    #[serde(default)]
    pub walnut: WalnutSection,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct WalnutSection {
    pub permissions: Option<Vec<String>>,
}

impl Manifest {
    pub fn permissions(&self, kind: PluginKind) -> Permissions {
        match (&self.walnut.permissions, kind) {
            (Some(list), _) => Permissions::from_names(list),
            (None, PluginKind::Wasm) => Permissions::default_set(),
            (None, _) => Permissions::all(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Discovered {
    /// Vault-relative plugin folder (`.obsidian/plugins/<dir>`), or `.obsidian/scripts` for scripts.
    pub dir: String,
    /// Vault-relative path of the entry point (`main.wasm`, `main.js`, `<name>.rhai`).
    pub entry: String,
    pub manifest: Manifest,
    pub kind: PluginKind,
    pub data_path: String,
    pub error: Option<String>,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !id.starts_with('.')
}

/// Find plugins in `.obsidian/plugins/*/manifest.json` and scripts in `.obsidian/scripts/*.rhai`.
pub fn discover(root: &Path) -> Vec<Discovered> {
    let mut out = Vec::new();
    let plugins = root.join(".obsidian/plugins");
    let mut dirs: Vec<_> = std::fs::read_dir(&plugins)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    dirs.sort();
    for dir in dirs {
        let base = format!(".obsidian/plugins/{dir}");
        let manifest_text = match std::fs::read_to_string(plugins.join(&dir).join("manifest.json")) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let kind =
            if plugins.join(&dir).join("main.wasm").is_file() { PluginKind::Wasm } else { PluginKind::Js };
        let entry = format!("{base}/{}", if kind == PluginKind::Wasm { "main.wasm" } else { "main.js" });
        let (manifest, mut error) = match serde_json::from_str::<Manifest>(&manifest_text) {
            Ok(m) => (m, None),
            Err(e) => (
                Manifest { id: dir.clone(), name: dir.clone(), ..Default::default() },
                Some(format!("manifest.json: {e}")),
            ),
        };
        if error.is_none() && !valid_id(&manifest.id) {
            error = Some(format!("invalid plugin id {:?}", manifest.id));
        } else if error.is_none() && manifest.id != dir {
            error = Some(format!("manifest id {:?} does not match folder name {dir:?}", manifest.id));
        } else if error.is_none()
            && !plugins
                .join(&dir)
                .join(if kind == PluginKind::Wasm { "main.wasm" } else { "main.js" })
                .is_file()
        {
            error = Some("no main.wasm or main.js".into());
        }
        let data_path = format!("{base}/data.json");
        out.push(Discovered { dir: base, entry, manifest, kind, data_path, error });
    }
    let scripts = root.join(".obsidian/scripts");
    let mut names: Vec<String> = std::fs::read_dir(&scripts)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".rhai").map(str::to_string))
        .collect();
    names.sort();
    for name in names {
        let id = format!("script-{name}");
        let error = (!valid_id(&id)).then(|| format!("invalid script name {name:?}"));
        out.push(Discovered {
            dir: ".obsidian/scripts".into(),
            entry: format!(".obsidian/scripts/{name}.rhai"),
            manifest: Manifest { id, name: name.clone(), ..Default::default() },
            kind: PluginKind::Script,
            data_path: format!(".obsidian/scripts/{name}.data.json"),
            error,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_plugins_and_scripts() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join(".obsidian/plugins/hello");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(
            p.join("manifest.json"),
            r#"{"id":"hello","name":"Hello","version":"1.0.0","walnut":{"permissions":["vault.write"]}}"#,
        )
        .unwrap();
        std::fs::write(p.join("main.wasm"), b"\0asm").unwrap();
        let bad = d.path().join(".obsidian/plugins/bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("manifest.json"), r#"{"id":"other","name":"x"}"#).unwrap();
        std::fs::write(bad.join("main.js"), "").unwrap();
        std::fs::create_dir_all(d.path().join(".obsidian/scripts")).unwrap();
        std::fs::write(d.path().join(".obsidian/scripts/tidy.rhai"), "").unwrap();
        let found = discover(d.path());
        assert_eq!(found.len(), 3);
        let bad = &found[0];
        assert!(bad.error.as_deref().unwrap().contains("does not match"));
        let hello = &found[1];
        assert_eq!(hello.kind, PluginKind::Wasm);
        assert!(hello.error.is_none());
        let perms = hello.manifest.permissions(hello.kind);
        assert!(perms.has(crate::Permission::VaultWrite) && !perms.has(crate::Permission::VaultRead));
        assert_eq!(found[2].manifest.id, "script-tidy");
    }
}
