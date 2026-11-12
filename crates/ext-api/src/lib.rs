//! Stable, versioned host API for every extension kind (WASM plugins, scripts, Obsidian JS plugins).
//!
//! Extensions talk to the app through one JSON call boundary: [`Api::call`] takes
//! `{"method": "...", "params": {...}}` and returns `{"ok": ...}` or `{"error": "..."}`. The app
//! implements [`Host`] for vault and editor access; commands, UI contributions and event
//! subscriptions live in the [`Registry`]. Every call is checked against the plugin's
//! [`Permissions`], and vault paths are confined to the vault.

mod api;
mod event;
mod manager;
mod manifest;
mod path;
mod registry;
pub mod testing;

pub use api::{Api, Host, NoteInfo, Selection, API_VERSION};
pub use event::Event;
pub use manager::{Extension, Extensions, PluginKind, PluginStatus};
pub use manifest::{discover, Discovered, Manifest};
pub use path::{guard_path, safe_rel};
pub use registry::{CommandDef, Notice, Registry, RibbonItem, StatusItem};

use serde::{Deserialize, Serialize};

/// A capability a plugin may be granted. Manifests list them in `rustsidian.permissions`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Permission {
    #[serde(rename = "vault.read")]
    VaultRead,
    #[serde(rename = "vault.write")]
    VaultWrite,
    #[serde(rename = "editor.read")]
    EditorRead,
    #[serde(rename = "editor.write")]
    EditorWrite,
    #[serde(rename = "ui")]
    Ui,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Permissions(Vec<Permission>);

impl Permissions {
    pub fn all() -> Permissions {
        Permissions(vec![
            Permission::VaultRead,
            Permission::VaultWrite,
            Permission::EditorRead,
            Permission::EditorWrite,
            Permission::Ui,
        ])
    }
    /// What a plugin gets when its manifest names no permissions: read-only plus UI.
    pub fn default_set() -> Permissions {
        Permissions(vec![Permission::VaultRead, Permission::EditorRead, Permission::Ui])
    }
    pub fn from_names(names: &[String]) -> Permissions {
        let list = names
            .iter()
            .filter_map(|n| serde_json::from_value(serde_json::Value::String(n.clone())).ok())
            .collect();
        Permissions(list)
    }
    pub fn has(&self, p: Permission) -> bool {
        self.0.contains(&p)
    }
}

/// Who is calling: the identity every call is attributed to and checked against.
#[derive(Clone, Debug)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub perms: Permissions,
    /// Vault-relative path of the plugin's settings file.
    pub data_path: String,
}
