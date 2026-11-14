//! The welcome screen shown when Rustsidian starts without a vault: recent vaults, a folder
//! browser to open or create a vault, and a sample vault for a first look.

use crate::anim::{Anims, FAST_MS, SLOW_MS};
use crate::{icons, recent};
use rustsidian_ui::{Dom, El, Fonts, NodeId, Pixmap, Theme};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use theme_engine::Env;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Home,
    /// Browse for an existing folder to open.
    Open,
    /// Browse for the folder a new vault is created in.
    Create,
}

#[derive(Clone, Debug)]
pub struct Folder {
    pub name: String,
    pub path: PathBuf,
    /// Has a `.obsidian` folder, so it has been used as a vault before.
    pub vault: bool,
}

pub struct Welcome {
    pub screen: Screen,
    pub recents: Vec<PathBuf>,
    /// The folder being browsed; `None` is the list of drives (Windows).
    pub dir: Option<PathBuf>,
    pub folders: Vec<Folder>,
    /// Name typed for a new vault.
    pub name: String,
    pub error: Option<String>,
    pub hover_action: Option<String>,
    pub scrolls: HashMap<String, f32>,
    pub anims: Anims,
    /// A note to show first in the vault that was just chosen (the sample vault's welcome note).
    pub open_note: Option<String>,
    theme: Theme,
}

const SAMPLE_NAME: &str = "Rustsidian Vault";
const SAMPLE: &[(&str, &str)] = &[
    ("Welcome.md", include_str!("sample/Welcome.md")),
    ("Getting started/Keyboard shortcuts.md", include_str!("sample/Keyboard shortcuts.md")),
    ("Getting started/Markdown basics.md", include_str!("sample/Markdown basics.md")),
    ("Getting started/Links and the graph.md", include_str!("sample/Links and the graph.md")),
];

fn drives() -> Vec<Folder> {
    (b'A'..=b'Z')
        .map(|l| PathBuf::from(format!("{}:\\", l as char)))
        .filter(|p| p.is_dir())
        .map(|p| Folder { name: p.to_string_lossy().into_owned(), path: p, vault: false })
        .collect()
}

fn list(dir: &Path) -> Vec<Folder> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<Folder> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name.starts_with('$') {
                return None;
            }
            let path = e.path();
            path.is_dir().then(|| Folder { vault: path.join(".obsidian").is_dir(), name, path })
        })
        .collect();
    out.sort_by_key(|f| f.name.to_lowercase());
    out
}

impl Default for Welcome {
    fn default() -> Self {
        Self::new()
    }
}

impl Welcome {
    pub fn new() -> Welcome {
        let mut theme = Theme::new();
        theme.add_sheet("base", crate::shell::BASE_CSS);
        let mut w = Welcome {
            screen: Screen::Home,
            recents: recent::load(),
            dir: None,
            folders: Vec::new(),
            name: String::new(),
            error: None,
            hover_action: None,
            scrolls: HashMap::new(),
            anims: Anims::default(),
            open_note: None,
            theme,
        };
        w.go(recent::home_dir());
        w
    }

    /// Browse a folder (`None` is the list of drives).
    fn go(&mut self, dir: Option<PathBuf>) {
        self.folders = match &dir {
            Some(d) => list(d),
            None => drives(),
        };
        self.dir = dir;
        self.scrolls.remove("folders");
        self.error = None;
    }

    fn show(&mut self, screen: Screen) {
        self.screen = screen;
        self.error = None;
        self.anims.start("screen", SLOW_MS);
    }

    /// Where the sample vault goes: Documents when there is one, else the home folder.
    fn sample_path() -> Option<PathBuf> {
        let home = recent::home_dir()?;
        let docs = home.join("Documents");
        Some(if docs.is_dir() { docs } else { home }.join(SAMPLE_NAME))
    }

    fn create_sample(&mut self) -> Option<PathBuf> {
        let Some(root) = Self::sample_path() else {
            self.error = Some("Cannot find your home folder to put the sample vault in.".to_string());
            return None;
        };
        for (rel, text) in SAMPLE {
            let file = root.join(rel);
            if file.exists() {
                continue;
            }
            let made = file
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&file, text));
            if let Err(e) = made {
                self.error = Some(format!("Cannot create {}: {e}", root.display()));
                return None;
            }
        }
        self.open_note = Some(SAMPLE[0].0.to_string());
        Some(root)
    }

    fn create_named(&mut self) -> Option<PathBuf> {
        let name = self.name.trim();
        let Some(dir) = &self.dir else {
            self.error = Some("Pick a folder to create the vault in first.".to_string());
            return None;
        };
        if name.is_empty() {
            self.error = Some("Type a name for the new vault.".to_string());
            return None;
        }
        if name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) || name.starts_with('.') {
            self.error = Some("That name has characters a folder name cannot have.".to_string());
            return None;
        }
        let root = dir.join(name);
        if root.exists() {
            self.error =
                Some(format!("{} already exists. Use \"Open folder as vault\" for it.", root.display()));
            return None;
        }
        if let Err(e) = std::fs::create_dir_all(&root) {
            self.error = Some(format!("Cannot create {}: {e}", root.display()));
            return None;
        }
        Some(root)
    }

    /// Run an action. Returns the vault to open when one was chosen.
    pub fn run(&mut self, action: &str) -> Option<PathBuf> {
        if let Some(i) = action.strip_prefix("recent:").and_then(|i| i.parse::<usize>().ok()) {
            return self.recents.get(i).cloned();
        }
        if let Some(i) = action.strip_prefix("forget:").and_then(|i| i.parse::<usize>().ok()) {
            if let Some(p) = self.recents.get(i) {
                recent::forget(p);
                self.recents = recent::load();
            }
            return None;
        }
        if let Some(i) = action.strip_prefix("enter:").and_then(|i| i.parse::<usize>().ok()) {
            if let Some(f) = self.folders.get(i) {
                let path = f.path.clone();
                self.go(Some(path));
            }
            return None;
        }
        match action {
            "welcome:open" => self.show(Screen::Open),
            "welcome:create" => {
                self.name.clear();
                self.show(Screen::Create);
            }
            "welcome:sample" => return self.create_sample(),
            "welcome:back" => self.show(Screen::Home),
            "welcome:up" => {
                let parent = self.dir.as_ref().and_then(|d| d.parent().map(Path::to_path_buf));
                if parent.is_some() || cfg!(windows) {
                    self.go(parent);
                }
            }
            "welcome:home" => self.go(recent::home_dir()),
            "welcome:choose" => match self.screen {
                Screen::Open => {
                    if self.dir.is_none() {
                        self.error = Some("Pick a folder first.".to_string());
                    }
                    return self.dir.clone();
                }
                Screen::Create => return self.create_named(),
                Screen::Home => {}
            },
            _ => {}
        }
        None
    }

    pub fn click(&mut self, dom: &Dom, x: f32, y: f32) -> Option<PathBuf> {
        let action = dom.action_at(x, y)?.1.to_string();
        self.run(&action)
    }

    /// A named key (not text). Returns the vault to open when one was chosen.
    pub fn key(&mut self, name: &str) -> Option<PathBuf> {
        match name {
            "Escape" if self.screen != Screen::Home => self.show(Screen::Home),
            "Enter" if self.screen != Screen::Home => return self.run("welcome:choose"),
            "Backspace" if self.screen == Screen::Create => {
                self.name.pop();
            }
            "Backspace" if self.screen == Screen::Open => {
                self.run("welcome:up");
            }
            _ => {}
        }
        None
    }

    pub fn text_input(&mut self, text: &str) {
        if self.screen == Screen::Create {
            self.name.extend(text.chars().filter(|c| !c.is_control()));
            self.error = None;
        }
    }

    pub fn wheel(&mut self, dom: &Dom, x: f32, y: f32, lines: f32) {
        let Some(node) = dom.scroller_at(x, y) else { return };
        if let Some(id) = dom.nodes[node].el.id.clone() {
            let cur = dom.nodes[node].scroll_y;
            self.scrolls.insert(id, (cur + lines * 40.0).clamp(0.0, dom.max_scroll(node)));
        }
    }

    /// The pointer moved. Returns true when the frame must be redrawn.
    pub fn hover(&mut self, dom: &Dom, x: f32, y: f32) -> bool {
        let action = dom.action_at(x, y).map(|(_, a)| a.to_string());
        let changed = action != self.hover_action;
        self.hover_action = action;
        changed
    }

    fn home(&self, dom: &mut Dom, main: NodeId) {
        for (icon, title, text, action) in [
            (
                "folder",
                "Open folder as vault",
                "Use a folder of Markdown notes, or an existing Obsidian vault.",
                "welcome:open",
            ),
            (
                "folder-plus",
                "Create new vault",
                "Start with an empty folder for your notes.",
                "welcome:create",
            ),
            ("help", "Try the sample vault", "A few notes that show how things work.", "welcome:sample"),
        ] {
            let row = dom.add(main, El::div().class("welcome-action").action(action));
            dom.add(row, icons::el(icon).class("welcome-action-icon"));
            let texts = dom.add(row, El::div().class("welcome-action-text"));
            dom.add(texts, El::div().class("welcome-action-title").text(title));
            dom.add(texts, El::div().class("welcome-action-description").text(text));
            dom.add(row, icons::el("chevron-right"));
        }
        if self.recents.is_empty() {
            return;
        }
        dom.add(main, El::div().class("welcome-section-title").text("Recent vaults"));
        let list = dom.add(main, El::div().class("welcome-list").id("recents"));
        for (i, p) in self.recents.iter().enumerate() {
            let row = dom.add(list, El::div().class("welcome-row").action(format!("recent:{i}")));
            dom.add(row, icons::el("vault"));
            let texts = dom.add(row, El::div().class("welcome-action-text"));
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            dom.add(texts, El::div().class("welcome-row-title").text(name));
            dom.add(
                texts,
                El::div().class("welcome-action-description").text(p.to_string_lossy().into_owned()),
            );
            let x = dom.add(row, El::div().class("clickable-icon").action(format!("forget:{i}")));
            dom.add(x, icons::el("x"));
        }
    }

    fn browser(&self, dom: &mut Dom, main: NodeId) {
        let create = self.screen == Screen::Create;
        let header = dom.add(main, El::div().class("welcome-browser-header"));
        let back = dom.add(header, El::div().class("clickable-icon").action("welcome:back"));
        dom.add(back, icons::el("arrow-left"));
        let title = if create { "Create new vault" } else { "Open folder as vault" };
        dom.add(header, El::div().class("welcome-browser-title").text(title));
        let bar = dom.add(main, El::div().class("welcome-path"));
        for (icon, action) in [("arrow-up", "welcome:up"), ("home", "welcome:home")] {
            let b = dom.add(bar, El::div().class("clickable-icon").action(action));
            dom.add(b, icons::el(icon));
        }
        let shown = self.dir.as_ref().map_or("This PC".to_string(), |d| d.to_string_lossy().into_owned());
        dom.add(bar, El::div().class("welcome-path-text").text(shown));
        let list = dom.add(main, El::div().class("welcome-list welcome-folders").id("folders"));
        if self.folders.is_empty() {
            dom.add(list, El::div().class("welcome-empty").text("No folders in here."));
        }
        for (i, f) in self.folders.iter().enumerate() {
            let row = dom.add(list, El::div().class("welcome-row").action(format!("enter:{i}")));
            let icon = if self.dir.is_none() {
                "drive"
            } else if f.vault {
                "vault"
            } else {
                "folder"
            };
            dom.add(row, icons::el(icon).class(if f.vault { "is-vault" } else { "" }));
            dom.add(row, El::div().class("welcome-row-title").text(f.name.clone()));
            if f.vault {
                dom.add(row, El::div().class("welcome-tag").text("vault"));
            }
            dom.add(row, icons::el("chevron-right"));
        }
        let footer = dom.add(main, El::div().class("welcome-footer"));
        if create {
            let shown =
                if self.name.is_empty() { "Vault name▏".to_string() } else { format!("{}▏", self.name) };
            dom.add(
                footer,
                El::div().class("welcome-input").class_if(self.name.is_empty(), "is-empty").text(shown),
            );
            dom.add(
                footer,
                El::div().class("welcome-button mod-cta").text("Create").action("welcome:choose"),
            );
        } else {
            let here =
                self.dir.as_ref().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned());
            dom.add(
                footer,
                El::div()
                    .class("welcome-footer-hint")
                    .text("Go into the folder that holds your notes, then open it."),
            );
            let label = here.map_or("Open this folder".to_string(), |n| format!("Open \"{n}\""));
            dom.add(footer, El::div().class("welcome-button mod-cta").text(label).action("welcome:choose"));
        }
    }

    pub fn build(&self) -> Dom {
        let mut dom = Dom::new();
        dom.nodes[0].el.classes.push("theme-dark".to_string());
        let t = self.anims.t("intro");
        let mut wrap = El::div().class("welcome");
        if t < 1.0 {
            wrap = wrap.style(&format!("opacity: {t:.3}"));
        }
        let wrap = dom.add(0, wrap);
        let mut card = El::div().class("welcome-card");
        if t < 1.0 {
            card = card.style(&format!("margin-top: {:.1}px", 18.0 * (1.0 - t)));
        }
        let card = dom.add(wrap, card);
        let brand = dom.add(card, El::div().class("welcome-brand"));
        dom.add(brand, El::div().class("welcome-logo").image(crate::view::logo()));
        dom.add(brand, El::div().class("welcome-name").text("Rustsidian"));
        dom.add(
            brand,
            El::div().class("welcome-version").text(format!("Version {}", env!("CARGO_PKG_VERSION"))),
        );
        dom.add(
            brand,
            El::div()
                .class("welcome-tagline")
                .text("Your notes are plain Markdown files in a folder you choose."),
        );
        let s = self.anims.t("screen");
        let mut main = El::div().class("welcome-main");
        if s < 1.0 {
            main = main.style(&format!("opacity: {s:.3}; padding-left: {:.1}px", 28.0 + 10.0 * (1.0 - s)));
        }
        let main = dom.add(card, main);
        match self.screen {
            Screen::Home => self.home(&mut dom, main),
            Screen::Open | Screen::Create => self.browser(&mut dom, main),
        }
        if let Some(e) = &self.error {
            dom.add(main, El::div().class("welcome-error").text(e.clone()));
        }
        for (id, y) in &self.scrolls {
            if let Some(n) = dom.by_id(id) {
                dom.nodes[n].scroll_y = *y;
            }
        }
        crate::view::mark_hover(&mut dom, self.hover_action.as_deref());
        dom
    }

    /// Build, style, lay out and paint the welcome screen into `pixmap`.
    pub fn render(&mut self, fonts: &mut Fonts, pixmap: &mut Pixmap, scale: f32) -> Dom {
        self.anims.tick();
        let mut dom = self.build();
        dom.style(&self.theme, Env { dark: true, reduced_motion: false });
        dom.layout(fonts, pixmap.width() as f32 / scale, pixmap.height() as f32 / scale);
        icons::paint(&mut dom);
        let scene = dom.paint();
        let clear = dom.nodes[0].style.background;
        rustsidian_ui::render(&scene, fonts, pixmap, scale, clear);
        dom
    }

    /// Start the opening animation (the window calls this once it can animate).
    pub fn intro(&mut self) {
        self.anims.enabled = true;
        self.anims.start("intro", SLOW_MS + FAST_MS);
    }
}
