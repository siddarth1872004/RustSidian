use crate::{Api, Event, Host, PluginInfo, Registry};
use serde::Serialize;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginKind {
    Wasm,
    Js,
    Script,
}

/// One row of the plugin list: what exists on disk and what happened when it was loaded.
#[derive(Clone, Debug, Serialize)]
pub struct PluginStatus {
    pub id: String,
    pub name: String,
    pub version: String,
    pub kind: PluginKind,
    pub enabled: bool,
    pub loaded: bool,
    pub error: Option<String>,
}

/// A running extension of any kind. Implemented by the WASM, script and JS hosts.
pub trait Extension {
    fn load(&mut self, api: &mut Api) -> Result<(), String>;
    fn event(&mut self, _event: &Event, _api: &mut Api) -> Result<(), String> {
        Ok(())
    }
    /// Run a command this extension registered; `local_id` is the id it registered.
    fn command(&mut self, local_id: &str, api: &mut Api) -> Result<(), String>;
    fn unload(&mut self, _api: &mut Api) {}
}

struct Slot {
    info: PluginInfo,
    ext: Box<dyn Extension>,
    failures: u32,
}

const MAX_FAILURES: u32 = 5;

/// Owns the registry and every loaded extension, and routes commands and events to them.
#[derive(Default)]
pub struct Extensions {
    pub reg: Registry,
    slots: Vec<Slot>,
}

impl Extensions {
    pub fn loaded(&self) -> impl Iterator<Item = &str> {
        self.slots.iter().map(|s| s.info.id.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty() && self.reg.commands.is_empty()
    }

    /// Load `ext` and run its `load`. On failure nothing it registered is kept.
    pub fn add(
        &mut self,
        host: &mut dyn Host,
        info: PluginInfo,
        mut ext: Box<dyn Extension>,
    ) -> Result<(), String> {
        let res = ext.load(&mut Api { reg: &mut self.reg, host });
        match res {
            Ok(()) => {
                self.slots.push(Slot { info, ext, failures: 0 });
                Ok(())
            }
            Err(e) => {
                self.reg.remove_plugin(&info.id);
                Err(e)
            }
        }
    }

    fn fail(&mut self, i: usize, host: &mut dyn Host, what: &str, err: String) {
        let slot = &mut self.slots[i];
        slot.failures += 1;
        let id = slot.info.id.clone();
        self.reg.push_notice(&id, format!("{id}: {what} failed: {err}"));
        self.reg.push_log(&id, format!("{what} failed: {err}"));
        if self.slots[i].failures >= MAX_FAILURES {
            self.reg.push_notice(&id, format!("{id} was disabled after repeated errors"));
            let mut slot = self.slots.remove(i);
            slot.ext.unload(&mut Api { reg: &mut self.reg, host });
            self.reg.remove_plugin(&id);
        }
    }

    pub fn emit(&mut self, host: &mut dyn Host, event: &Event) {
        let mut i = 0;
        while i < self.slots.len() {
            let id = self.slots[i].info.id.clone();
            if self.reg.is_subscribed(&id, event.name()) {
                let slot = &mut self.slots[i];
                match slot.ext.event(event, &mut Api { reg: &mut self.reg, host }) {
                    Ok(()) => slot.failures = 0,
                    Err(e) => {
                        let before = self.slots.len();
                        self.fail(i, host, event.name(), e);
                        if self.slots.len() < before {
                            continue;
                        }
                    }
                }
            }
            i += 1;
        }
    }

    /// Run a plugin command by its full id. Returns false if no plugin owns it.
    pub fn run_command(&mut self, host: &mut dyn Host, id: &str) -> bool {
        let Some(owner) = self.reg.command(id).map(|c| c.plugin.clone()) else { return false };
        let Some(i) = self.slots.iter().position(|s| s.info.id == owner) else { return false };
        let local = id.strip_prefix(&format!("{owner}:")).unwrap_or(id).to_string();
        let slot = &mut self.slots[i];
        match slot.ext.command(&local, &mut Api { reg: &mut self.reg, host }) {
            Ok(()) => slot.failures = 0,
            Err(e) => self.fail(i, host, "command", e),
        }
        true
    }

    pub fn unload_all(&mut self, host: &mut dyn Host) {
        for mut slot in std::mem::take(&mut self.slots) {
            slot.ext.unload(&mut Api { reg: &mut self.reg, host });
            self.reg.remove_plugin(&slot.info.id);
        }
    }
}
