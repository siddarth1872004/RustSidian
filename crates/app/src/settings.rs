//! Settings window: Editor, Files & links, Appearance, Hotkeys and About. Every control writes
//! the same `.obsidian/*.json` key Obsidian uses (other keys are left as they are) and takes
//! effect at once.

use crate::shell::{default_keymap, load_theme, App, Key, COMMANDS};
use serde_json::{json, Value};
use vault_core::config::JsonFile;
use vault_core::fsio::TrashMode;
use walnut_ui::{Dom, El, NodeId};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Editor,
    Files,
    Appearance,
    Hotkeys,
    About,
}

const SECTIONS: [(Section, &str, &str); 5] = [
    (Section::Editor, "editor", "Editor"),
    (Section::Files, "files", "Files & links"),
    (Section::Appearance, "appearance", "Appearance"),
    (Section::Hotkeys, "hotkeys", "Hotkeys"),
    (Section::About, "about", "About"),
];

pub struct SettingsUi {
    pub section: Section,
    /// The text field being edited: (setting key, text so far).
    pub edit: Option<(String, String)>,
    /// The command whose hotkey is waiting for a key press.
    pub capture: Option<String>,
    pub filter: String,
    pub themes: Vec<String>,
    pub snippets: Vec<String>,
}

impl SettingsUi {
    fn new(app: &App) -> SettingsUi {
        let mut s = SettingsUi {
            section: Section::Editor,
            edit: None,
            capture: None,
            filter: String::new(),
            themes: Vec::new(),
            snippets: Vec::new(),
        };
        s.scan(app);
        s
    }

    fn scan(&mut self, app: &App) {
        let list = |sub: &str| -> Vec<std::path::PathBuf> {
            let mut v: Vec<_> = std::fs::read_dir(app.config.dir.join(sub))
                .map(|d| d.flatten().map(|e| e.path()).collect())
                .unwrap_or_default();
            v.sort();
            v
        };
        self.themes = list("themes")
            .into_iter()
            .filter(|p| p.join("theme.css").is_file())
            .filter_map(|p| Some(p.file_name()?.to_str()?.to_string()))
            .collect();
        self.snippets = list("snippets")
            .into_iter()
            .filter(|p| p.extension().is_some_and(|e| e == "css"))
            .filter_map(|p| Some(p.file_stem()?.to_str()?.to_string()))
            .collect();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum File {
    App,
    Appearance,
}

enum Control {
    Toggle(bool),
    Choice(&'static str, &'static [(&'static str, &'static str)]),
    Text,
    Number(f64, f64, f64),
}

struct Item {
    section: Section,
    file: File,
    key: &'static str,
    name: &'static str,
    desc: &'static str,
    control: Control,
}

use Control::{Choice, Number, Text, Toggle};
use Section::{Appearance, Editor, Files};

const ITEMS: &[Item] = &[
    Item {
        section: Editor,
        file: File::App,
        key: "vimMode",
        name: "Vim key bindings",
        desc: "Use Vim-style keys in the editor.",
        control: Toggle(false),
    },
    Item {
        section: Editor,
        file: File::App,
        key: "readableLineLength",
        name: "Readable line length",
        desc: "Limit the width of lines in the editor.",
        control: Toggle(false),
    },
    Item {
        section: Editor,
        file: File::App,
        key: "propertiesInDocument",
        name: "Properties in document",
        desc: "Show front matter as a properties table, as raw source, or not at all.",
        control: Choice("visible", &[("visible", "Visible"), ("source", "Source"), ("hidden", "Hidden")]),
    },
    Item {
        section: Editor,
        file: File::App,
        key: "useTab",
        name: "Indent using tabs",
        desc: "Tab inserts a tab character instead of spaces.",
        control: Toggle(true),
    },
    Item {
        section: Editor,
        file: File::App,
        key: "tabSize",
        name: "Tab indent size",
        desc: "Number of spaces one indent level is worth.",
        control: Number(4.0, 1.0, 16.0),
    },
    Item {
        section: Editor,
        file: File::App,
        key: "autoPairBrackets",
        name: "Auto pair brackets",
        desc: "Typing ( [ { or a quote also inserts the closing one.",
        control: Toggle(true),
    },
    Item {
        section: Editor,
        file: File::App,
        key: "autoPairMarkdown",
        name: "Auto pair Markdown syntax",
        desc: "Typing * _ ` or ~ also inserts the closing marker.",
        control: Toggle(true),
    },
    Item {
        section: Editor,
        file: File::App,
        key: "smartIndentList",
        name: "Smart lists",
        desc: "Enter continues a list item and Backspace removes an empty one.",
        control: Toggle(true),
    },
    Item {
        section: Files,
        file: File::App,
        key: "newFileLocation",
        name: "Default location for new notes",
        desc: "Where Create new note puts the file.",
        control: Choice(
            "root",
            &[
                ("root", "Vault folder"),
                ("current", "Same folder as current file"),
                ("folder", "In the folder specified below"),
            ],
        ),
    },
    Item {
        section: Files,
        file: File::App,
        key: "newFileFolderPath",
        name: "Folder for new notes",
        desc: "Used when the location above is a specific folder.",
        control: Text,
    },
    Item {
        section: Files,
        file: File::App,
        key: "newLinkFormat",
        name: "New link format",
        desc: "How links are written when a file is renamed or moved.",
        control: Choice(
            "shortest",
            &[
                ("shortest", "Shortest path when possible"),
                ("relative", "Relative path to file"),
                ("absolute", "Absolute path in vault"),
            ],
        ),
    },
    Item {
        section: Files,
        file: File::App,
        key: "trashOption",
        name: "Deleted files",
        desc: "What happens to a note you delete.",
        control: Choice(
            "local",
            &[
                ("system", "Move to system trash"),
                ("local", "Move to vault .trash folder"),
                ("none", "Permanently delete"),
            ],
        ),
    },
    Item {
        section: Appearance,
        file: File::Appearance,
        key: "theme",
        name: "Base color scheme",
        desc: "Light or dark base for the interface.",
        control: Choice("obsidian", &[("obsidian", "Dark"), ("moonstone", "Light"), ("system", "Adapt to system")]),
    },
    Item {
        section: Appearance,
        file: File::Appearance,
        key: "baseFontSize",
        name: "Font size",
        desc: "Size of text in notes, in pixels.",
        control: Number(16.0, 10.0, 30.0),
    },
];

fn item(key: &str) -> Option<&'static Item> {
    ITEMS.iter().find(|i| i.key == key)
}

fn number_json(n: f64) -> Value {
    if n.fract() == 0.0 {
        json!(n as i64)
    } else {
        json!(n)
    }
}

/// Editor options from `app.json`.
pub fn editor_settings(config: &vault_core::config::ObsidianConfig) -> editor_core::Settings {
    let a = &config.app;
    editor_core::Settings {
        auto_pair_brackets: a.bool("autoPairBrackets", true),
        auto_pair_markdown: a.bool("autoPairMarkdown", true),
        smart_lists: a.bool("smartIndentList", true),
        tab_size: a.get("tabSize").and_then(Value::as_u64).map_or(4, |n| n.clamp(1, 16) as usize),
        use_tabs: a.bool("useTab", true),
    }
}

fn hotkey_json(k: &Key) -> Value {
    let mut mods = Vec::new();
    if k.ctrl {
        mods.push("Mod");
    }
    if k.alt {
        mods.push("Alt");
    }
    if k.shift {
        mods.push("Shift");
    }
    let key = if k.key.chars().count() == 1 { k.key.to_uppercase() } else { k.key.clone() };
    json!([{ "modifiers": mods, "key": key }])
}

impl App {
    fn setting_file(&self, f: File) -> &JsonFile {
        match f {
            File::App => &self.config.app,
            File::Appearance => &self.config.appearance,
        }
    }

    fn save_setting(&mut self, f: File, key: &str, v: Value) {
        let file = match f {
            File::App => &mut self.config.app,
            File::Appearance => &mut self.config.appearance,
        };
        file.set(key, v);
        let r = file.save();
        if let Err(e) = r {
            self.notice = Some(format!("Cannot save {}: {e}", self.setting_file(f).path.display()));
        }
        self.setting_changed(key);
    }

    /// Make a changed setting take effect.
    fn setting_changed(&mut self, key: &str) {
        match key {
            "vimMode" => {
                let on = self.config.app.bool("vimMode", false);
                self.vim = on;
                for t in &mut self.tabs {
                    t.vim = on.then(Default::default);
                }
            }
            "useTab" | "tabSize" | "autoPairBrackets" | "autoPairMarkdown" | "smartIndentList" => {
                for t in &mut self.tabs {
                    t.editor.settings = editor_settings(&self.config);
                }
            }
            "newLinkFormat" => self.store.index.link_format = self.config.link_format(),
            "trashOption" => self.store.trash_mode = self.config.trash_option(),
            "theme" => self.dark = self.config.color_scheme().unwrap_or(true),
            "cssTheme" | "baseFontSize" | "enabledCssSnippets" => self.theme = load_theme(&self.config),
            "propertiesInDocument" => self.prop_edit = None,
            _ => {}
        }
    }

    /// Settings-derived state for a note tab that was just opened.
    pub(crate) fn init_new_tab(&mut self) {
        let s = editor_settings(&self.config);
        let i = self.tabs.len().saturating_sub(1);
        if let Some(t) = self.tabs.get_mut(i) {
            t.editor.settings = s;
        }
        self.park_cursor_after_front(i);
    }

    pub(crate) fn config_changed(&mut self) {
        for k in ["vimMode", "useTab", "newLinkFormat", "trashOption", "propertiesInDocument"] {
            self.setting_changed(k);
        }
        self.setting_changed("theme");
    }

    /// Folder that `Create new note` uses.
    pub(crate) fn new_note_dir(&self) -> String {
        match self.config.app.str("newFileLocation") {
            Some("current") => {
                self.tab().map(|t| vault_core::path::parent(&t.path).to_string()).unwrap_or_default()
            }
            Some("folder") => vault_core::path::normalize(self.config.app.str("newFileFolderPath").unwrap_or("")),
            _ => String::new(),
        }
    }

    fn set_hotkey(&mut self, cmd: &str, key: Option<Key>) {
        let v = key.as_ref().map_or_else(|| json!([]), hotkey_json);
        self.config.hotkeys.set(cmd, v);
        self.after_hotkey_change();
        if let Some(k) = key {
            let seq = vec![k];
            let other = self.keymap.iter().find(|(s, c)| *s == seq && c != cmd).map(|(_, c)| c.clone());
            if let Some(name) = other.and_then(|c| COMMANDS.iter().find(|x| x.0 == c).map(|x| x.1)) {
                self.notice = Some(format!("This key is also bound to \"{name}\""));
            }
        }
    }

    fn after_hotkey_change(&mut self) {
        if let Err(e) = self.config.hotkeys.save() {
            self.notice = Some(format!("Cannot save hotkeys.json: {e}"));
        }
        self.keymap = default_keymap(&self.config);
    }

    fn commit_setting_edit(&mut self) {
        let Some((key, text)) = self.settings.as_mut().and_then(|s| s.edit.take()) else { return };
        if key == "hk-filter" {
            if let Some(s) = &mut self.settings {
                s.filter = text;
            }
            return;
        }
        let Some(it) = item(&key) else { return };
        let v = match it.control {
            Number(_, lo, hi) => match text.trim().parse::<f64>() {
                Ok(n) if n.is_finite() => number_json(n.clamp(lo, hi)),
                _ => {
                    self.notice = Some(format!("\"{}\" is not a number", text.trim()));
                    return;
                }
            },
            _ => json!(text.trim()),
        };
        self.save_setting(it.file, it.key, v);
    }

    pub(crate) fn run_settings(&mut self, command: &str) -> bool {
        if command == "app:open-settings" {
            if self.settings.is_some() {
                self.settings = None;
            } else {
                self.prompt = None;
                self.prop_edit = None;
                self.settings = Some(SettingsUi::new(self));
                self.scrolls.remove("settings-content");
            }
            return true;
        }
        let Some(rest) = command.strip_prefix("settings:") else { return false };
        if self.settings.is_none() {
            return true;
        }
        let (op, arg) = rest.split_once(':').unwrap_or((rest, ""));
        let keeps_edit = matches!(op, "noop") || (op == "edit" && self.edit_key() == Some(arg));
        if !keeps_edit {
            self.commit_setting_edit();
        }
        match op {
            "close" => self.settings = None,
            "section" => {
                if let Some(&(sec, ..)) = SECTIONS.iter().find(|s| s.1 == arg) {
                    if let Some(s) = &mut self.settings {
                        s.section = sec;
                        s.capture = None;
                    }
                    self.scrolls.remove("settings-content");
                }
            }
            "toggle" => {
                if let Some(it) = item(arg) {
                    let Toggle(default) = it.control else { return true };
                    let now = self.setting_file(it.file).bool(it.key, default);
                    self.save_setting(it.file, it.key, json!(!now));
                }
            }
            "cycle" => {
                if let Some((it, Choice(default, opts))) = item(arg).map(|i| (i, &i.control)) {
                    let cur = self.setting_file(it.file).str(it.key).unwrap_or(default).to_string();
                    let at = opts.iter().position(|o| o.0 == cur).map_or(0, |p| p + 1);
                    self.save_setting(it.file, it.key, json!(opts[at % opts.len()].0));
                }
            }
            "edit" => {
                let text = if arg == "hk-filter" {
                    self.settings.as_ref().map(|s| s.filter.clone()).unwrap_or_default()
                } else {
                    match item(arg) {
                        Some(it) => match self.setting_file(it.file).get(it.key) {
                            Some(Value::String(s)) => s.clone(),
                            Some(Value::Number(n)) => n.as_f64().map(vault_core_num).unwrap_or_default(),
                            _ => match it.control {
                                Number(d, ..) => vault_core_num(d),
                                _ => String::new(),
                            },
                        },
                        None => return true,
                    }
                };
                if let Some(s) = &mut self.settings {
                    s.edit = Some((arg.to_string(), text));
                }
            }
            "css-theme" => {
                let mut opts = vec![String::new()];
                opts.extend(self.settings.iter().flat_map(|s| s.themes.clone()));
                let cur = self.config.css_theme().unwrap_or("").to_string();
                if !opts.contains(&cur) {
                    opts.push(cur.clone());
                }
                let at = opts.iter().position(|o| *o == cur).map_or(0, |p| p + 1);
                self.save_setting(File::Appearance, "cssTheme", json!(opts[at % opts.len()]));
            }
            "snippet" => {
                let mut on: Vec<Value> = self
                    .config
                    .appearance
                    .get("enabledCssSnippets")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                match on.iter().position(|v| v.as_str() == Some(arg)) {
                    Some(p) => {
                        on.remove(p);
                    }
                    None => on.push(json!(arg)),
                }
                self.save_setting(File::Appearance, "enabledCssSnippets", Value::Array(on));
            }
            "reload" => {
                self.run("app:reload-theme");
                self.config_changed();
                let ui = SettingsUi::new(self);
                if let Some(s) = &mut self.settings {
                    s.themes = ui.themes;
                    s.snippets = ui.snippets;
                }
            }
            "perf" => self.perf = !self.perf,
            "hk" => {
                if let Some(s) = &mut self.settings {
                    s.capture = (s.capture.as_deref() != Some(arg)).then(|| arg.to_string());
                }
            }
            "hk-reset" => {
                self.config.hotkeys.remove(arg);
                self.after_hotkey_change();
            }
            _ => {}
        }
        true
    }

    fn edit_key(&self) -> Option<&str> {
        self.settings.as_ref()?.edit.as_ref().map(|e| e.0.as_str())
    }

    /// Key press while a property or a settings control has the keyboard. `None` lets the
    /// regular shortcuts see the key.
    pub(crate) fn modal_key(&mut self, key: &Key) -> Option<bool> {
        if let Some(r) = self.prop_key(key) {
            return Some(r);
        }
        let s = self.settings.as_mut()?;
        let plain = !key.ctrl && !key.alt && !key.shift;
        if let Some(cmd) = s.capture.clone() {
            match key.key.as_str() {
                "Escape" if plain => s.capture = None,
                "Backspace" | "Delete" if plain => {
                    s.capture = None;
                    self.set_hotkey(&cmd, None);
                }
                k if k.chars().count() == 1 && !key.ctrl && !key.alt => {
                    self.notice = Some("A hotkey for a character needs Ctrl or Alt".into());
                }
                _ => {
                    s.capture = None;
                    self.set_hotkey(&cmd, Some(key.clone()));
                }
            }
            return Some(true);
        }
        if let Some((_, buf)) = &mut s.edit {
            match key.key.as_str() {
                "Escape" => s.edit = None,
                "Enter" | "Tab" => self.commit_setting_edit(),
                "Backspace" => {
                    buf.pop();
                }
                _ => {}
            }
            return Some(true);
        }
        if key.key == "Escape" {
            self.settings = None;
            return Some(true);
        }
        if key.ctrl || key.alt {
            return None;
        }
        Some(true)
    }

    pub(crate) fn modal_text(&mut self, text: &str) -> bool {
        if self.prop_text(text) {
            return true;
        }
        let Some(s) = &mut self.settings else { return false };
        if let Some((_, buf)) = &mut s.edit {
            buf.push_str(text);
        }
        true
    }
}

fn vault_core_num(n: f64) -> String {
    md_engine::frontmatter::fmt_number(n)
}

fn row(dom: &mut Dom, parent: NodeId, name: &str, desc: &str) -> NodeId {
    let item = dom.add(parent, El::div().class("setting-item"));
    let info = dom.add(item, El::div().class("setting-item-info"));
    dom.add(info, El::div().class("setting-item-name").text(name));
    if !desc.is_empty() {
        dom.add(info, El::div().class("setting-item-description").text(desc));
    }
    dom.add(item, El::div().class("setting-item-control"))
}

fn toggle(dom: &mut Dom, ctl: NodeId, on: bool, action: String) {
    let c = dom.add(ctl, El::div().class("checkbox-container").class_if(on, "is-enabled").action(action));
    dom.add(c, El::div().class("checkbox-thumb"));
}

fn button(dom: &mut Dom, ctl: NodeId, label: &str, action: String, cta: bool) {
    dom.add(ctl, El::div().class("setting-button").class_if(cta, "mod-cta").text(label).action(action));
}

fn input(dom: &mut Dom, ctl: NodeId, key: &str, shown: &str, edit: Option<&(String, String)>) {
    let editing = edit.filter(|e| e.0 == key);
    let text = editing.map_or_else(|| shown.to_string(), |e| format!("{}▏", e.1));
    dom.add(
        ctl,
        El::div()
            .class("setting-input")
            .class_if(editing.is_some(), "is-editing")
            .text(text)
            .action(format!("settings:edit:{key}")),
    );
}

fn heading(dom: &mut Dom, parent: NodeId, text: &str) {
    dom.add(parent, El::div().class("setting-item-heading").text(text));
}

fn items_view(app: &App, dom: &mut Dom, parent: NodeId, ui: &SettingsUi, section: Section) {
    for it in ITEMS.iter().filter(|i| i.section == section) {
        let ctl = row(dom, parent, it.name, it.desc);
        let file = app.setting_file(it.file);
        match &it.control {
            Toggle(d) => toggle(dom, ctl, file.bool(it.key, *d), format!("settings:toggle:{}", it.key)),
            Choice(d, opts) => {
                let cur = file.str(it.key).unwrap_or(d);
                let label = opts.iter().find(|o| o.0 == cur).map_or(cur, |o| o.1);
                dom.add(
                    ctl,
                    El::div().class("dropdown").text(format!("{label} ▾")).action(format!("settings:cycle:{}", it.key)),
                );
            }
            Text => input(dom, ctl, it.key, file.str(it.key).unwrap_or(""), ui.edit.as_ref()),
            Number(d, ..) => {
                let n = file.get(it.key).and_then(Value::as_f64).unwrap_or(*d);
                input(dom, ctl, it.key, &vault_core_num(n), ui.edit.as_ref());
            }
        }
    }
}

fn appearance_view(app: &App, dom: &mut Dom, parent: NodeId, ui: &SettingsUi) {
    items_view(app, dom, parent, ui, Section::Appearance);
    let theme = app.config.css_theme().unwrap_or("");
    let ctl = row(dom, parent, "Theme", "Community theme from .obsidian/themes. Click to cycle.");
    let label = if theme.is_empty() { "Default" } else { theme };
    dom.add(ctl, El::div().class("dropdown").text(format!("{label} ▾")).action("settings:css-theme"));
    heading(dom, parent, "CSS snippets");
    let ctl = row(dom, parent, "Reload", "Read themes, snippets and settings from disk again.");
    button(dom, ctl, "Reload", "settings:reload".into(), false);
    if ui.snippets.is_empty() {
        dom.add(parent, El::div().class("setting-item-description").text("No snippets in .obsidian/snippets."));
    }
    let enabled = app.config.enabled_snippets();
    for s in &ui.snippets {
        let ctl = row(dom, parent, &format!("{s}.css"), "");
        toggle(dom, ctl, enabled.contains(&s.as_str()), format!("settings:snippet:{s}"));
    }
}

fn hotkeys_view(app: &App, dom: &mut Dom, parent: NodeId, ui: &SettingsUi) {
    let ctl = row(dom, parent, "Filter", "Show commands whose name contains this text.");
    let shown = if ui.filter.is_empty() { "Filter commands…" } else { &ui.filter };
    input(dom, ctl, "hk-filter", shown, ui.edit.as_ref());
    let needle = ui.filter.to_lowercase();
    let custom = app.config.hotkeys.value.as_object();
    for (id, name, _) in COMMANDS.iter().filter(|c| c.1.to_lowercase().contains(&needle)) {
        let ctl = row(dom, parent, name, "");
        let capturing = ui.capture.as_deref() == Some(id);
        let bound = app.hotkey_for(id);
        let label = if capturing {
            "Press a key (Esc cancels, Backspace clears)".to_string()
        } else if bound.is_empty() {
            "Blank".to_string()
        } else {
            bound
        };
        dom.add(
            ctl,
            El::div()
                .class("setting-hotkey")
                .class_if(capturing, "is-capturing")
                .class_if(label == "Blank", "is-blank")
                .text(label)
                .action(format!("settings:hk:{id}")),
        );
        if custom.is_some_and(|o| o.contains_key(*id)) {
            dom.add(ctl, El::div().class("setting-hotkey-reset").text("↺").action(format!("settings:hk-reset:{id}")));
        }
    }
}

fn about_view(app: &App, dom: &mut Dom, parent: NodeId) {
    let ctl = row(dom, parent, "Rustwalnut", "Obsidian-compatible knowledge base.");
    dom.add(ctl, El::div().class("setting-value").text(format!("v{}", env!("CARGO_PKG_VERSION"))));
    let ctl = row(dom, parent, "Vault", "");
    dom.add(ctl, El::div().class("setting-value").text(app.store.root.display().to_string()));
    let ctl = row(dom, parent, "Notes", "");
    dom.add(ctl, El::div().class("setting-value").text(app.store.index.notes().count().to_string()));
    let ctl = row(dom, parent, "Performance overlay", "Show the frame time and node count in the corner.");
    toggle(dom, ctl, app.perf, "settings:perf".into());
}

/// The settings window, drawn over everything else.
pub fn modal(app: &App, dom: &mut Dom) {
    let Some(ui) = &app.settings else { return };
    let backdrop = dom.add(0, El::div().class("modal-container mod-dim").action("settings:close"));
    let modal = dom.add(backdrop, El::div().class("modal mod-settings").action("settings:noop"));
    let tabs = dom.add(modal, El::div().class("vertical-tab-header"));
    dom.add(tabs, El::div().class("vertical-tab-header-group-title").text("Options"));
    for (sec, slug, name) in SECTIONS {
        dom.add(
            tabs,
            El::div()
                .class("vertical-tab-nav-item")
                .class_if(sec == ui.section, "is-active")
                .text(name)
                .action(format!("settings:section:{slug}")),
        );
    }
    let container = dom.add(modal, El::div().class("vertical-tab-content-container"));
    let content = dom.add(container, El::div().class("vertical-tab-content").id("settings-content"));
    let title = SECTIONS.iter().find(|s| s.0 == ui.section).map_or("", |s| s.2);
    heading(dom, content, title);
    match ui.section {
        Section::Appearance => appearance_view(app, dom, content, ui),
        Section::Hotkeys => hotkeys_view(app, dom, content, ui),
        Section::About => about_view(app, dom, content),
        s => items_view(app, dom, content, ui, s),
    }
    dom.add(container, El::div().class("setting-footer").text("Changes are saved to .obsidian as you make them.  Esc closes."));
    let _ = TrashMode::Vault;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_json_uses_obsidian_names() {
        let k = Key::parse("Ctrl+Shift+h").unwrap();
        assert_eq!(hotkey_json(&k), json!([{ "modifiers": ["Mod", "Shift"], "key": "H" }]));
        let k = Key::parse("Alt+ArrowUp").unwrap();
        assert_eq!(hotkey_json(&k), json!([{ "modifiers": ["Alt"], "key": "ArrowUp" }]));
    }

    #[test]
    fn items_have_unique_keys() {
        let mut keys: Vec<_> = ITEMS.iter().map(|i| i.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), ITEMS.len());
    }
}
