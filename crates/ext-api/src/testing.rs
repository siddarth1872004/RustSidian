//! An in-memory [`Host`] for tests of extension hosts.

use crate::{Host, NoteInfo, Selection};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct MemHost {
    pub root: PathBuf,
    pub files: BTreeMap<String, String>,
    pub active: Option<String>,
    pub editor: String,
    pub sel: (usize, usize),
    pub ran: Vec<String>,
}

impl MemHost {
    pub fn new(root: &Path) -> MemHost {
        MemHost {
            root: root.to_path_buf(),
            files: BTreeMap::new(),
            active: None,
            editor: String::new(),
            sel: (0, 0),
            ran: Vec::new(),
        }
    }
}

impl Host for MemHost {
    fn vault_root(&self) -> &Path {
        &self.root
    }
    fn list_files(&mut self) -> Vec<String> {
        self.files.keys().cloned().collect()
    }
    fn read_note(&mut self, path: &str) -> Result<String, String> {
        self.files.get(path).cloned().ok_or_else(|| format!("not found: {path}"))
    }
    fn write_note(&mut self, path: &str, text: &str) -> Result<(), String> {
        self.files.insert(path.to_string(), text.to_string());
        Ok(())
    }
    fn delete_note(&mut self, path: &str) -> Result<(), String> {
        self.files.remove(path).map(|_| ()).ok_or_else(|| format!("not found: {path}"))
    }
    fn rename_note(&mut self, from: &str, to: &str) -> Result<(), String> {
        let t = self.files.remove(from).ok_or_else(|| format!("not found: {from}"))?;
        self.files.insert(to.to_string(), t);
        Ok(())
    }
    fn note_info(&mut self, path: &str) -> Option<NoteInfo> {
        let text = self.files.get(path)?;
        let tags = text.split_whitespace().filter_map(|w| w.strip_prefix('#')).map(str::to_string).collect();
        Some(NoteInfo { path: path.to_string(), tags, ..Default::default() })
    }
    fn backlinks(&mut self, _path: &str) -> Vec<String> {
        Vec::new()
    }
    fn resolve_link(&mut self, target: &str, _source: &str) -> Option<String> {
        self.files.keys().find(|k| k.trim_end_matches(".md") == target).cloned()
    }
    fn active_file(&mut self) -> Option<String> {
        self.active.clone()
    }
    fn editor_text(&mut self) -> Option<String> {
        self.active.as_ref().map(|_| self.editor.clone())
    }
    fn selection(&mut self) -> Option<Selection> {
        self.active.as_ref()?;
        let (from, to) = self.sel;
        Some(Selection { from, to, text: self.editor.get(from..to).unwrap_or("").to_string() })
    }
    fn replace_range(&mut self, from: usize, to: usize, text: &str) -> Result<(), String> {
        if from > to
            || to > self.editor.len()
            || !self.editor.is_char_boundary(from)
            || !self.editor.is_char_boundary(to)
        {
            return Err("range out of bounds".into());
        }
        self.editor.replace_range(from..to, text);
        Ok(())
    }
    fn run_command(&mut self, id: &str) -> Result<(), String> {
        self.ran.push(id.to_string());
        Ok(())
    }
}
