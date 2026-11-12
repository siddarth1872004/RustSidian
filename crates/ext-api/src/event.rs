use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "name", rename_all = "kebab-case")]
pub enum Event {
    FileOpen { path: String },
    Modify { path: String },
    Create { path: String },
    Delete { path: String },
    Rename { path: String, old_path: String },
    LayoutReady,
}

impl Event {
    pub fn name(&self) -> &'static str {
        match self {
            Event::FileOpen { .. } => "file-open",
            Event::Modify { .. } => "modify",
            Event::Create { .. } => "create",
            Event::Delete { .. } => "delete",
            Event::Rename { .. } => "rename",
            Event::LayoutReady => "layout-ready",
        }
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}
