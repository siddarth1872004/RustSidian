//! Right-click menus for explorer rows, tabs and the editor, and the file operations they offer
//! (rename, new folder, make a copy, delete, copy path, reveal in the system file manager), plus
//! cut, copy and paste.

use crate::shell::{App, Focus, Key, PromptKind, ViewMode};
use crate::{clipboard, view, Frame};
use editor_core::{EditKind, Sel};
use rustsidian_ui::Fonts;
use vault_core::{fsio, path};

pub const ITEM_H: f32 = 28.0;
pub const SEPARATOR_H: f32 = 9.0;
pub const WIDTH: f32 = 248.0;

/// One menu row. A row with an empty action is a separator.
#[derive(Clone, Debug)]
pub struct MenuItem {
    pub label: String,
    pub icon: &'static str,
    pub action: String,
    pub hotkey: String,
    /// Destructive actions are shown in the warning color.
    pub warning: bool,
}

#[derive(Clone, Debug)]
pub struct Menu {
    pub x: f32,
    pub y: f32,
    pub items: Vec<MenuItem>,
    /// The row picked with the arrow keys.
    pub selected: Option<usize>,
}

impl Menu {
    pub fn height(&self) -> f32 {
        8.0 + self.items.iter().map(|i| if i.action.is_empty() { SEPARATOR_H } else { ITEM_H }).sum::<f32>()
    }
}

struct Builder<'a> {
    app: &'a App,
    items: Vec<MenuItem>,
}

impl Builder<'_> {
    fn item(&mut self, icon: &'static str, label: &str, action: impl Into<String>) -> &mut Self {
        let action = action.into();
        let hotkey = self.app.hotkey_for(&action);
        self.items.push(MenuItem { label: label.into(), icon, action, hotkey, warning: false });
        self
    }
    fn warning(&mut self, icon: &'static str, label: &str, action: impl Into<String>) -> &mut Self {
        self.item(icon, label, action);
        if let Some(last) = self.items.last_mut() {
            last.warning = true;
        }
        self
    }
    fn separator(&mut self) -> &mut Self {
        if self.items.last().is_some_and(|i| !i.action.is_empty()) {
            self.items.push(MenuItem {
                label: String::new(),
                icon: "",
                action: String::new(),
                hotkey: String::new(),
                warning: false,
            });
        }
        self
    }
}

impl App {
    /// Right click at window coordinates: open the menu for what is under the pointer.
    pub fn context_menu(&mut self, frame: &Frame, fonts: &mut Fonts, x: f32, y: f32) {
        self.menu = None;
        if self.prompt.is_some() || self.settings.is_some() {
            return;
        }
        let action = frame.dom.action_at(x, y).map(|(_, a)| a.to_string());
        let mut b = Builder { app: self, items: Vec::new() };
        match action.as_deref() {
            Some(a) if a.starts_with("open:") => file_items(&mut b, &a["open:".len()..]),
            Some(a) if a.starts_with("toggle-folder:") => folder_items(&mut b, &a["toggle-folder:".len()..]),
            Some(a) if a.starts_with("tab:") || a.starts_with("close-tab:") => {
                let i: usize = a.rsplit(':').next().and_then(|i| i.parse().ok()).unwrap_or(0);
                tab_items(&mut b, i);
            }
            Some("graph:focus") => {
                b.item("x", "Close", "graph:close");
            }
            Some("canvas:focus") => {
                b.item("x", "Close", "canvas:close");
            }
            Some(_) => return,
            None => {
                let in_files =
                    frame.dom.by_id("files").is_some_and(|n| frame.dom.nodes[n].rect.contains(x, y));
                if in_files {
                    folder_items(&mut b, "");
                } else if let Some(off) = view::offset_at(&frame.dom, &frame.info, fonts, x, y) {
                    drop(b);
                    self.focus = Focus::Editor;
                    if let Some(t) = self.tab_mut() {
                        // Keep a selection the click falls inside, so "Cut" and "Copy" act on it.
                        let inside = t
                            .editor
                            .buf
                            .sels
                            .iter()
                            .any(|s| !s.is_empty() && s.start() <= off && off <= s.end());
                        if !inside {
                            t.editor.buf.set_sels(vec![Sel::cursor(off)]);
                        }
                    }
                    let mut b = Builder { app: self, items: Vec::new() };
                    editor_items(&mut b);
                    let items = b.items;
                    return self.show_menu(x, y, items);
                } else {
                    return;
                }
            }
        }
        let items = b.items;
        self.show_menu(x, y, items);
    }

    fn show_menu(&mut self, x: f32, y: f32, mut items: Vec<MenuItem>) {
        while items.last().is_some_and(|i| i.action.is_empty()) {
            items.pop();
        }
        if items.is_empty() {
            return;
        }
        self.menu = Some(Menu { x, y, items, selected: None });
    }

    /// While a menu is open, a click either picks a row or just closes the menu.
    /// Returns true when the click was taken by the menu.
    pub(crate) fn menu_click(&mut self, frame: &Frame, x: f32, y: f32) -> bool {
        let Some(menu) = self.menu.take() else { return false };
        let picked = frame
            .dom
            .action_at(x, y)
            .and_then(|(_, a)| a.strip_prefix("menu:"))
            .and_then(|i| i.parse::<usize>().ok())
            .and_then(|i| menu.items.get(i))
            .map(|i| i.action.clone());
        if let Some(action) = picked {
            self.run(&action);
        }
        true
    }

    /// Keys while a menu is open: arrows move, Enter picks, Escape closes. Any other key closes
    /// the menu and is then handled as usual.
    pub(crate) fn menu_key(&mut self, key: &Key) -> Option<bool> {
        let menu = self.menu.as_mut()?;
        let rows: Vec<usize> = (0..menu.items.len()).filter(|&i| !menu.items[i].action.is_empty()).collect();
        let at = menu.selected.and_then(|s| rows.iter().position(|&r| r == s));
        match key.key.as_str() {
            "Escape" => self.menu = None,
            "ArrowDown" => menu.selected = rows.get(at.map_or(0, |a| (a + 1) % rows.len())).copied(),
            "ArrowUp" => {
                menu.selected =
                    rows.get(at.map_or(rows.len() - 1, |a| (a + rows.len() - 1) % rows.len())).copied()
            }
            "Enter" => {
                let action = menu.selected.and_then(|s| menu.items.get(s)).map(|i| i.action.clone());
                self.menu = None;
                if let Some(a) = action {
                    self.run(&a);
                }
            }
            _ => {
                self.menu = None;
                return None;
            }
        }
        Some(true)
    }

    /// File operations and clipboard commands. Returns false for commands it does not know.
    pub(crate) fn run_file_action(&mut self, command: &str) -> bool {
        let (verb, arg) = command.split_once(':').unwrap_or((command, ""));
        match verb {
            "rename-file" => {
                let initial = if self.is_folder(arg) { path::file_name(arg) } else { path::stem(arg) };
                let initial = initial.to_string();
                self.open_name_prompt(PromptKind::Rename, arg, &initial);
            }
            "rename" => {
                let Some((from, name)) = arg.split_once('|') else { return true };
                self.rename_path(from, name);
            }
            "new-folder-in" => self.open_name_prompt(PromptKind::NewFolder, arg, ""),
            "mkdir" => {
                let Some((parent, name)) = arg.split_once('|') else { return true };
                self.make_folder(parent, name);
            }
            "new-note-in" => self.new_note_in(arg),
            "duplicate" => self.duplicate(arg),
            "delete" => self.delete_path(arg),
            "copy-path" => {
                let p = arg.to_string();
                self.put_clip(p);
            }
            "copy-link" => {
                let link = format!("[[{}]]", path::strip_md(arg));
                self.put_clip(link);
            }
            "reveal" => self.reveal(arg),
            "close-others" => {
                if let Ok(i) = arg.parse::<usize>() {
                    self.active = i.min(self.tabs.len().saturating_sub(1));
                    self.run("workspace:close-others");
                }
            }
            "close-right" => {
                if let Ok(i) = arg.parse::<usize>() {
                    for j in (i + 1..self.tabs.len()).rev() {
                        self.close_tab(j);
                    }
                }
            }
            _ => match command {
                "file-explorer:new-folder" => {
                    let dir = self.new_note_dir();
                    self.open_name_prompt(PromptKind::NewFolder, &dir, "");
                }
                "workspace:edit-file-title" => {
                    if let Some(p) = self.tab().map(|t| t.path.clone()) {
                        self.run(&format!("rename-file:{p}"));
                    }
                }
                "workspace:copy-path" => {
                    if let Some(p) = self.tab().map(|t| t.path.clone()) {
                        self.put_clip(p);
                    }
                }
                "file-explorer:reveal-active-file" => self.reveal_in_explorer(),
                "editor:copy" => self.copy(false),
                "editor:cut" => self.copy(true),
                "editor:paste" => self.paste(),
                "editor:insert-wikilink" => self.insert_wikilink(),
                _ => return false,
            },
        }
        true
    }

    fn is_folder(&self, rel: &str) -> bool {
        self.store.index.id(rel).is_none() && self.store.abs(rel).is_dir()
    }

    fn rename_path(&mut self, from: &str, name: &str) {
        let clean = path::sanitize_file_name(name.trim(), path::SanitizePolicy::Strip);
        if clean.is_empty() {
            self.notice = Some("A name is needed".into());
            return;
        }
        let folder = self.is_folder(from);
        let ext = path::extension(from);
        let file = if folder || ext.is_empty() || path::extension(&clean).eq_ignore_ascii_case(ext) {
            clean
        } else {
            format!("{clean}.{ext}")
        };
        let to = path::join(path::parent(from), &file);
        if to == from {
            return;
        }
        // Unsaved edits would be lost when the files move, so everything is saved first.
        self.save_all();
        match self.store.rename(from, &to) {
            Ok(plan) => {
                for (old, new) in &plan.moves {
                    for t in self.tabs.iter_mut().filter(|t| t.path.eq_ignore_ascii_case(old)) {
                        t.path = new.clone();
                    }
                    self.plugins.push(ext_api::Event::Rename { path: new.clone(), old_path: old.clone() });
                }
                let edited: Vec<String> = plan.edits.iter().map(|e| e.path.clone()).collect();
                self.reload_tabs(&edited);
                self.files_changed(&[]);
                self.notice = Some(match edited.len() {
                    0 => format!("Renamed to {file}"),
                    1 => format!("Renamed to {file}; updated links in 1 file"),
                    n => format!("Renamed to {file}; updated links in {n} files"),
                });
            }
            Err(e) => self.notice = Some(format!("Cannot rename {from}: {e}")),
        }
    }

    /// Replace the text of open tabs whose files were rewritten on disk.
    fn reload_tabs(&mut self, paths: &[String]) {
        for p in paths {
            let Some(i) = self.tabs.iter().position(|t| t.path.eq_ignore_ascii_case(p)) else { continue };
            if let Ok((text, stamp)) = self.store.read(p) {
                let tab = &mut self.tabs[i];
                let head = tab.editor.buf.sels[0].head.min(text.len());
                tab.editor = editor_core::Editor::new(&text);
                tab.editor.buf.set_sels(vec![Sel::cursor(head)]);
                tab.stamp = stamp;
            }
        }
    }

    /// The explorer lists folders through the files in them, so a new folder starts with a note.
    fn make_folder(&mut self, parent: &str, name: &str) {
        let clean = path::sanitize_file_name(name.trim(), path::SanitizePolicy::Strip);
        if clean.is_empty() {
            self.notice = Some("A name is needed".into());
            return;
        }
        let dir = path::join(parent, &clean);
        if self.store.abs(&dir).exists() {
            self.notice = Some(format!("{dir} already exists"));
            return;
        }
        if let Err(e) = std::fs::create_dir_all(self.store.abs(&dir)) {
            self.notice = Some(format!("Cannot create {dir}: {e}"));
            return;
        }
        self.collapsed.remove(&dir);
        self.tree = None;
        self.new_note_in(&dir);
    }

    fn new_note_in(&mut self, dir: &str) {
        let rel = (0..)
            .map(|n| if n == 0 { "Untitled.md".to_string() } else { format!("Untitled {n}.md") })
            .map(|name| path::join(dir, &name))
            .find(|p| self.store.index.id(p).is_none() && !self.store.abs(p).exists())
            .unwrap_or_default();
        match self.store.create(&rel, "") {
            Ok(_) => {
                self.plugins.push(ext_api::Event::Create { path: rel.clone() });
                self.collapsed.remove(dir);
                self.tree = None;
                self.files_changed(&[]);
                self.open_note(&rel);
            }
            Err(e) => self.notice = Some(format!("Cannot create note: {e}")),
        }
    }

    fn duplicate(&mut self, rel: &str) {
        self.save_all();
        let text = match self.store.read(rel) {
            Ok((t, _)) => t,
            Err(e) => {
                self.notice = Some(format!("Cannot read {rel}: {e}"));
                return;
            }
        };
        let (dir, stem, ext) = (path::parent(rel), path::stem(rel), path::extension(rel));
        let copy = (1..)
            .map(|n| path::join(dir, &format!("{stem} {n}.{ext}")))
            .find(|p| self.store.index.id(p).is_none() && !self.store.abs(p).exists())
            .unwrap_or_default();
        match self.store.create(&copy, &text) {
            Ok(_) => {
                self.plugins.push(ext_api::Event::Create { path: copy.clone() });
                self.files_changed(&[]);
                self.open_note(&copy);
            }
            Err(e) => self.notice = Some(format!("Cannot copy {rel}: {e}")),
        }
    }

    fn delete_path(&mut self, rel: &str) {
        let prefix = format!("{}/", rel.to_lowercase());
        let gone = |p: &str| p.eq_ignore_ascii_case(rel) || p.to_lowercase().starts_with(&prefix);
        match self.store.delete(rel) {
            Ok(dest) => {
                let before = self.tabs.len();
                let active_gone = self.tab().is_some_and(|t| gone(&t.path));
                self.tabs.retain(|t| !gone(&t.path));
                if active_gone || self.tabs.len() != before {
                    self.active = self.active.min(self.tabs.len().saturating_sub(1));
                }
                self.plugins.push(ext_api::Event::Delete { path: rel.to_string() });
                self.files_changed(&[]);
                self.notice = Some(match dest {
                    Some(_) => format!("Moved {rel} to trash"),
                    None => format!("Deleted {rel}"),
                });
            }
            Err(e) => self.notice = Some(format!("Cannot delete {rel}: {e}")),
        }
    }

    /// Show a file or folder in the system file manager.
    fn reveal(&mut self, rel: &str) {
        let abs = self.store.abs(rel);
        let shown = abs.display().to_string();
        let result = if cfg!(windows) {
            std::process::Command::new("explorer")
                .arg(format!("/select,{}", shown.replace('/', "\\")))
                .spawn()
        } else if cfg!(target_os = "macos") {
            std::process::Command::new("open").arg("-R").arg(&abs).spawn()
        } else {
            let dir = if abs.is_dir() {
                abs.clone()
            } else {
                abs.parent().map(|p| p.to_path_buf()).unwrap_or(abs.clone())
            };
            std::process::Command::new("xdg-open").arg(dir).spawn()
        };
        if let Err(e) = result {
            self.notice = Some(format!("Cannot open the file manager: {e}"));
        }
    }

    /// Expand the folders above the current file and scroll the explorer to it.
    fn reveal_in_explorer(&mut self) {
        let Some(p) = self.tab().map(|t| t.path.clone()) else { return };
        let mut dir = path::parent(&p).to_string();
        while !dir.is_empty() {
            self.collapsed.remove(&dir);
            dir = path::parent(&dir).to_string();
        }
        self.tree = None;
        self.run("file-explorer:open");
        self.refresh_derived();
        if let Some(i) = self.tree_rows().iter().position(|r| r.path.eq_ignore_ascii_case(&p)) {
            let y = (i as f32 * crate::shell::TREE_ROW_HEIGHT - 120.0).max(0.0);
            self.scrolls.insert("files".into(), y);
        }
    }

    fn put_clip(&mut self, text: String) {
        if self.system_clipboard {
            if let Err(e) = clipboard::set(&text) {
                self.notice = Some(format!("Copied inside Rustsidian only: {e}"));
            }
        }
        self.clip = text;
    }

    fn take_clip(&mut self) -> String {
        if self.system_clipboard {
            match clipboard::get() {
                Ok(t) => return t,
                Err(e) => self.notice = Some(format!("Cannot read the system clipboard: {e}")),
            }
        }
        self.clip.clone()
    }

    /// Copy (or cut) the selections. With nothing selected the whole line is taken, as in most
    /// code editors.
    fn copy(&mut self, cut: bool) {
        if self.focus != Focus::Editor || self.show_graph || self.canvas_active() {
            return;
        }
        let now = fsio::now_ms();
        let Some(t) = self.tabs.get_mut(self.active) else { return };
        let buf = &t.editor.buf;
        let text = if buf.sels.iter().all(|s| s.is_empty()) {
            let line = buf.line_of(buf.sels[0].head);
            let (start, end) = (buf.line_start(line), buf.line_end(line));
            if cut && t.mode != ViewMode::Reading {
                let next = if line + 1 < buf.line_count() { buf.line_start(line + 1) } else { end };
                let from = if next == end && line > 0 { buf.line_end(line - 1) } else { start };
                t.editor.buf.set_sels(vec![Sel::new(from, next)]);
            }
            format!("{}\n", buf_slice(&t.editor.buf, start, end))
        } else {
            let parts: Vec<String> = buf
                .sels
                .iter()
                .filter(|s| !s.is_empty())
                .map(|s| buf_slice(buf, s.start(), s.end()))
                .collect();
            parts.join("\n")
        };
        if cut && t.mode != ViewMode::Reading {
            t.editor.replace_selections("", EditKind::Other, now);
            t.last_edit_ms = now;
        }
        self.put_clip(text);
        if cut {
            self.scroll_to_cursor();
        }
    }

    fn paste(&mut self) {
        let text = self.take_clip();
        if text.is_empty() {
            return;
        }
        match self.focus {
            Focus::Prompt => {
                let line = text.lines().next().unwrap_or("").to_string();
                if let Some(p) = &mut self.prompt {
                    p.query.push_str(&line);
                    p.selected = 0;
                }
                self.refresh_prompt();
            }
            Focus::Search => self.search_query.push_str(text.lines().next().unwrap_or("")),
            Focus::Editor if self.canvas_active() => self.canvas_text(&text),
            Focus::Editor => {
                let now = fsio::now_ms();
                let shown = !self.show_graph;
                let Some(t) = self.tabs.get_mut(self.active).filter(|t| shown && t.mode != ViewMode::Reading)
                else {
                    return;
                };
                t.editor.replace_selections(&text, EditKind::Other, now);
                t.last_edit_ms = now;
                self.scroll_to_cursor();
            }
        }
    }

    /// Wrap the selection in `[[ ]]`, or insert an empty link with the cursor inside it.
    fn insert_wikilink(&mut self) {
        let now = fsio::now_ms();
        let Some(t) = self.tabs.get_mut(self.active).filter(|t| t.mode != ViewMode::Reading) else { return };
        let sel = t.editor.buf.sels[0];
        let inner = buf_slice(&t.editor.buf, sel.start(), sel.end());
        t.editor.buf.set_sels(vec![sel]);
        t.editor.replace_selections(&format!("[[{inner}]]"), EditKind::Other, now);
        if inner.is_empty() {
            let at = t.editor.buf.sels[0].head.saturating_sub(2);
            t.editor.buf.set_sels(vec![Sel::cursor(at)]);
        }
        t.last_edit_ms = now;
    }
}

fn buf_slice(buf: &editor_core::Buffer, start: usize, end: usize) -> String {
    buf.slice(start, end).into_owned()
}

fn file_items(b: &mut Builder<'_>, p: &str) {
    b.item("note", "Open", format!("open:{p}"));
    b.separator();
    b.item("copy", "Make a copy", format!("duplicate:{p}"));
    b.item("edit", "Rename…", format!("rename-file:{p}"));
    b.separator();
    b.item("link", "Copy link", format!("copy-link:{p}"));
    b.item("clipboard", "Copy path", format!("copy-path:{p}"));
    b.item("folder", "Show in system explorer", format!("reveal:{p}"));
    b.separator();
    b.warning("trash", "Delete", format!("delete:{p}"));
}

fn folder_items(b: &mut Builder<'_>, dir: &str) {
    b.item("note-plus", "New note", format!("new-note-in:{dir}"));
    b.item("folder-plus", "New folder", format!("new-folder-in:{dir}"));
    if dir.is_empty() {
        b.separator();
        b.item("collapse", "Collapse all", "file-explorer:collapse-all");
        b.item("folder", "Show in system explorer", "reveal:");
        return;
    }
    b.separator();
    b.item("edit", "Rename…", format!("rename-file:{dir}"));
    b.item("clipboard", "Copy path", format!("copy-path:{dir}"));
    b.item("folder", "Show in system explorer", format!("reveal:{dir}"));
    b.separator();
    b.warning("trash", "Delete", format!("delete:{dir}"));
}

fn tab_items(b: &mut Builder<'_>, i: usize) {
    let count = b.app.tabs.len();
    let p = b.app.tabs.get(i).map(|t| t.path.clone()).unwrap_or_default();
    b.item("x", "Close", format!("close-tab:{i}"));
    if count > 1 {
        b.item("x", "Close others", format!("close-others:{i}"));
    }
    if i + 1 < count {
        b.item("arrow-right", "Close tabs to the right", format!("close-right:{i}"));
    }
    b.item("x", "Close all", "workspace:close-all");
    b.separator();
    if !p.is_empty() {
        b.item("edit", "Rename…", format!("rename-file:{p}"));
        b.item("link", "Copy link", format!("copy-link:{p}"));
        b.item("clipboard", "Copy path", format!("copy-path:{p}"));
        b.item("folder", "Show in system explorer", format!("reveal:{p}"));
        b.separator();
    }
    b.item("arrow-up", "Reopen closed tab", "workspace:undo-close-pane");
}

fn editor_items(b: &mut Builder<'_>) {
    let reading = b.app.tab().is_some_and(|t| t.mode == ViewMode::Reading);
    if b.app.link_under_cursor().is_some() {
        b.item("link", "Open link", "editor:follow-link");
        b.separator();
    }
    if !reading {
        b.item("scissors", "Cut", "editor:cut");
    }
    b.item("copy", "Copy", "editor:copy");
    if !reading {
        b.item("clipboard", "Paste", "editor:paste");
        b.separator();
        b.item("link", "Add internal link", "editor:insert-wikilink");
        b.item("bold", "Bold", "editor:toggle-bold");
        b.item("italic", "Italic", "editor:toggle-italics");
        b.item("strikethrough", "Strikethrough", "editor:toggle-strikethrough");
        b.item("highlight", "Highlight", "editor:toggle-highlight");
        b.item("code", "Code", "editor:toggle-code");
        b.separator();
        b.item("check", "Toggle checkbox", "editor:toggle-checklist-status");
        b.item("table", "Insert table", "editor:insert-table");
    }
    b.separator();
    b.item("select", "Select all", "editor:select-all");
    b.item(
        if reading { "edit" } else { "book" },
        if reading { "Edit" } else { "Reading view" },
        "markdown:toggle-preview",
    );
}
