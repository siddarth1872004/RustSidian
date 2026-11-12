use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandDef {
    /// Full id, `<plugin>:<id>`.
    pub id: String,
    pub name: String,
    pub plugin: String,
    pub hotkey: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusItem {
    pub plugin: String,
    pub id: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RibbonItem {
    pub plugin: String,
    pub id: String,
    pub icon: String,
    pub title: String,
    pub command: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub plugin: String,
    pub text: String,
}

/// Everything extensions have contributed to the app.
#[derive(Default)]
pub struct Registry {
    pub commands: Vec<CommandDef>,
    pub status: Vec<StatusItem>,
    pub ribbon: Vec<RibbonItem>,
    pub notices: Vec<Notice>,
    pub logs: Vec<(String, String)>,
    subs: HashSet<(String, String)>,
}

pub(crate) const MAX_NOTICES: usize = 32;
pub(crate) const MAX_LOGS: usize = 256;
pub(crate) const MAX_ITEMS_PER_PLUGIN: usize = 64;

impl Registry {
    pub fn subscribe(&mut self, plugin: &str, event: &str) {
        self.subs.insert((plugin.to_string(), event.to_string()));
    }
    pub fn is_subscribed(&self, plugin: &str, event: &str) -> bool {
        self.subs.contains(&(plugin.to_string(), event.to_string()))
    }
    pub fn command(&self, id: &str) -> Option<&CommandDef> {
        self.commands.iter().find(|c| c.id == id)
    }
    pub fn take_notices(&mut self) -> Vec<Notice> {
        std::mem::take(&mut self.notices)
    }
    pub(crate) fn push_notice(&mut self, plugin: &str, text: String) {
        if self.notices.len() < MAX_NOTICES {
            self.notices.push(Notice { plugin: plugin.to_string(), text });
        }
    }
    pub(crate) fn push_log(&mut self, plugin: &str, text: String) {
        if self.logs.len() >= MAX_LOGS {
            self.logs.remove(0);
        }
        self.logs.push((plugin.to_string(), text));
    }
    /// Remove everything a plugin contributed.
    pub fn remove_plugin(&mut self, plugin: &str) {
        self.commands.retain(|c| c.plugin != plugin);
        self.status.retain(|s| s.plugin != plugin);
        self.ribbon.retain(|r| r.plugin != plugin);
        self.subs.retain(|(p, _)| p != plugin);
    }
}
