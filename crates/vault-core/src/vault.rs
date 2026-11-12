//! In-memory vault index: files, name lookup, link resolution, link graph, rename planning.

pub use crate::cache::Meta;
use crate::meta::{Link, LinkKind, NoteMeta};
use crate::path::{self, FileKind};
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

pub type FileId = u32;

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: Arc<str>,
    /// Modification time in nanoseconds since the Unix epoch.
    pub mtime: i64,
    pub size: u64,
    pub kind: FileKind,
    pub(crate) meta: Option<Meta>,
}

impl FileEntry {
    pub fn has_meta(&self) -> bool {
        self.meta.is_some()
    }
    /// The note's metadata. Notes loaded from the persisted index are decoded on each call, so
    /// call this once per use rather than once per field.
    pub fn meta(&self) -> Option<Arc<NoteMeta>> {
        self.meta.as_ref().map(Meta::get)
    }
    /// Like `meta`, but only the selected [`crate::cache::parts`] are guaranteed to be filled in.
    pub fn meta_parts(&self, parts: u8) -> Option<Arc<NoteMeta>> {
        self.meta.as_ref().map(|m| m.get_parts(parts))
    }
    /// Visit the targets of the note's links without decoding the rest of its metadata.
    pub fn link_targets(&self, mut f: impl FnMut(&str)) {
        if let Some(m) = &self.meta {
            m.link_targets(&mut f);
        }
    }
}

/// How newly generated links are written. Mirrors Obsidian's "New link format".
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LinkFormat {
    #[default]
    Shortest,
    Relative,
    Absolute,
}

#[derive(Clone, Debug, Default)]
pub struct LinkGraph {
    /// Outgoing resolved links per file: (target, number of links).
    pub out: Vec<Vec<(FileId, u32)>>,
    /// Files linking to each file.
    pub back: Vec<Vec<FileId>>,
    /// Unresolved link targets per file.
    pub unresolved: Vec<Vec<Box<str>>>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TextEdit {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileEdits {
    /// Path of the file after the rename.
    pub path: String,
    pub edits: Vec<TextEdit>,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct RenamePlan {
    /// (old path, new path) for every file that moves.
    pub moves: Vec<(String, String)>,
    pub edits: Vec<FileEdits>,
}

/// Apply non-overlapping edits to a text.
pub fn apply_edits(text: &str, edits: &[TextEdit]) -> String {
    let mut sorted: Vec<&TextEdit> = edits.iter().collect();
    sorted.sort_by_key(|e| e.start);
    let mut out = String::with_capacity(text.len() + 64);
    let mut pos = 0;
    for e in sorted {
        if e.start < pos || e.end > text.len() {
            continue;
        }
        out.push_str(&text[pos..e.start]);
        out.push_str(&e.text);
        pos = e.end;
    }
    out.push_str(&text[pos..]);
    out
}

#[derive(Default)]
pub struct Vault {
    files: Vec<Option<FileEntry>>,
    free: Vec<FileId>,
    by_lower: HashMap<Box<str>, FileId>,
    by_name: HashMap<Box<str>, Vec<FileId>>,
    graph: Option<Arc<LinkGraph>>,
    pub link_format: LinkFormat,
}

impl Vault {
    pub fn new() -> Vault {
        Vault::default()
    }

    pub fn len(&self) -> usize {
        self.by_lower.len()
    }
    pub fn is_empty(&self) -> bool {
        self.by_lower.is_empty()
    }
    /// Upper bound of file ids (for dense per-file tables).
    pub fn id_bound(&self) -> usize {
        self.files.len()
    }

    pub fn id(&self, p: &str) -> Option<FileId> {
        self.by_lower.get(p.to_lowercase().as_str()).copied()
    }
    pub fn entry(&self, id: FileId) -> Option<&FileEntry> {
        self.files.get(id as usize)?.as_ref()
    }
    pub fn get(&self, p: &str) -> Option<&FileEntry> {
        self.entry(self.id(p)?)
    }
    pub fn iter(&self) -> impl Iterator<Item = (FileId, &FileEntry)> {
        self.files.iter().enumerate().filter_map(|(i, f)| f.as_ref().map(|f| (i as FileId, f)))
    }
    pub fn notes(&self) -> impl Iterator<Item = (FileId, &FileEntry)> {
        self.iter().filter(|(_, f)| f.kind == FileKind::Markdown)
    }

    /// Insert or replace a file. Replacing only the metadata of an existing file keeps its id.
    pub fn upsert(&mut self, p: &str, mtime: i64, size: u64, meta: Option<NoteMeta>) -> FileId {
        self.upsert_meta(p, mtime, size, meta.map(|m| Meta::Owned(Arc::new(m))))
    }

    pub fn upsert_meta(&mut self, p: &str, mtime: i64, size: u64, meta: Option<Meta>) -> FileId {
        self.graph = None;
        if let Some(id) = self.id(p) {
            if let Some(f) = self.files[id as usize].as_mut() {
                f.mtime = mtime;
                f.size = size;
                f.meta = meta;
            }
            return id;
        }
        let entry = FileEntry { path: Arc::from(p), mtime, size, kind: path::kind_of(p), meta };
        let id = match self.free.pop() {
            Some(id) => {
                self.files[id as usize] = Some(entry);
                id
            }
            None => {
                self.files.push(Some(entry));
                (self.files.len() - 1) as FileId
            }
        };
        self.index_names(id, p);
        id
    }

    fn index_names(&mut self, id: FileId, p: &str) {
        let lower = p.to_lowercase();
        self.by_name.entry(path::file_name(&lower).into()).or_default().push(id);
        self.by_lower.insert(lower.into(), id);
    }

    fn unindex_names(&mut self, id: FileId, p: &str) {
        let lower = p.to_lowercase();
        if let Some(v) = self.by_name.get_mut(path::file_name(&lower)) {
            v.retain(|&x| x != id);
            if v.is_empty() {
                self.by_name.remove(path::file_name(&lower));
            }
        }
        self.by_lower.remove(lower.as_str());
    }

    pub fn remove(&mut self, p: &str) -> Option<FileEntry> {
        let id = self.id(p)?;
        self.graph = None;
        let entry = self.files[id as usize].take()?;
        self.unindex_names(id, &entry.path);
        self.free.push(id);
        Some(entry)
    }

    /// Move an entry in the index, keeping its id and metadata.
    pub fn move_entry(&mut self, old: &str, new: &str) -> Option<FileId> {
        let id = self.id(old)?;
        self.graph = None;
        let old_path = self.files[id as usize].as_ref()?.path.clone();
        self.unindex_names(id, &old_path);
        if let Some(f) = self.files[id as usize].as_mut() {
            f.path = Arc::from(new);
            f.kind = path::kind_of(new);
        }
        self.index_names(id, new);
        Some(id)
    }

    /// Resolve a link target written in `source` to a file.
    ///
    /// Order: exact vault path, path relative to the source folder, then file name anywhere in
    /// the vault (same folder as the source first, then the shallowest, then alphabetical).
    /// Matching is case-insensitive and `.md` is optional.
    pub fn resolve(&self, target: &str, source: &str) -> Option<FileId> {
        let t = target.trim();
        if t.is_empty() {
            return self.id(source);
        }
        let t = t.replace('\\', "/");
        let explicit_rel = t.starts_with("./") || t.starts_with("../");
        let absolute = t.starts_with('/');
        let src_dir = path::parent(source);
        let norm = if explicit_rel { path::join(src_dir, &t) } else { path::normalize(&t) };
        if norm.is_empty() {
            return None;
        }
        let lower = norm.to_lowercase();
        let with_md = format!("{lower}.md");
        if let Some(&id) = self.by_lower.get(with_md.as_str()).or_else(|| self.by_lower.get(lower.as_str())) {
            return Some(id);
        }
        if explicit_rel || absolute {
            return None;
        }
        if !src_dir.is_empty() {
            let rel = path::join(src_dir, &norm).to_lowercase();
            let rel_md = format!("{rel}.md");
            if let Some(&id) = self.by_lower.get(rel_md.as_str()).or_else(|| self.by_lower.get(rel.as_str()))
            {
                return Some(id);
            }
        }
        let src_dir_lower = src_dir.to_lowercase();
        for suffix in [&with_md, &lower] {
            let Some(cands) = self.by_name.get(path::file_name(suffix)) else { continue };
            let mut best: Option<(bool, usize, &str, FileId)> = None;
            for &id in cands {
                let Some(f) = self.entry(id) else { continue };
                let p: &str = &f.path;
                let pl = p.to_lowercase();
                let matches = pl == **suffix
                    || (pl.len() > suffix.len()
                        && pl.ends_with(suffix.as_str())
                        && pl.as_bytes()[pl.len() - suffix.len() - 1] == b'/');
                if !matches {
                    continue;
                }
                let same_dir = path::parent(&pl) == src_dir_lower;
                let key = (!same_dir, p.matches('/').count(), p, id);
                if best.is_none_or(|b| (key.0, key.1, key.2) < (b.0, b.1, b.2)) {
                    best = Some(key);
                }
            }
            if let Some(b) = best {
                return Some(b.3);
            }
        }
        None
    }

    /// The link graph, computed on first use after a change.
    pub fn graph(&mut self) -> Arc<LinkGraph> {
        if let Some(g) = &self.graph {
            return g.clone();
        }
        let n = self.files.len();
        type PerFile = (Vec<(FileId, u32)>, Vec<Box<str>>);
        let per_file: Vec<PerFile> = (0..n)
            .into_par_iter()
            .map(|i| {
                let mut out: Vec<(FileId, u32)> = Vec::new();
                let mut unresolved: Vec<Box<str>> = Vec::new();
                let Some(f) = self.files[i].as_ref() else { return (out, unresolved) };
                f.link_targets(|target| match self.resolve(target, &f.path) {
                    Some(t) => match out.iter_mut().find(|(x, _)| *x == t) {
                        Some(slot) => slot.1 += 1,
                        None => out.push((t, 1)),
                    },
                    None => {
                        if !unresolved.iter().any(|u| **u == *target) {
                            unresolved.push(target.into());
                        }
                    }
                });
                (out, unresolved)
            })
            .collect();
        let mut g = LinkGraph {
            out: Vec::with_capacity(n),
            back: vec![Vec::new(); n],
            unresolved: Vec::with_capacity(n),
        };
        for (i, (out, unresolved)) in per_file.into_iter().enumerate() {
            for &(t, _) in &out {
                if t as usize != i {
                    g.back[t as usize].push(i as FileId);
                }
            }
            g.out.push(out);
            g.unresolved.push(unresolved);
        }
        let g = Arc::new(g);
        self.graph = Some(g.clone());
        g
    }

    /// Text to write for a link to `target` from `source`, in the given format.
    /// `markdown` produces a Markdown link destination (extension kept, percent-encoded).
    pub fn link_text(&self, target: FileId, source: &str, format: LinkFormat, markdown: bool) -> String {
        let Some(f) = self.entry(target) else { return String::new() };
        let full: &str = &f.path;
        let shape = |p: &str| -> String {
            if markdown {
                path::percent_encode(p)
            } else if f.kind == FileKind::Markdown {
                path::strip_md(p).to_string()
            } else {
                p.to_string()
            }
        };
        let candidate = match format {
            LinkFormat::Absolute => full.to_string(),
            LinkFormat::Relative => path::relative(path::parent(source), full),
            LinkFormat::Shortest => path::file_name(full).to_string(),
        };
        if self.resolve(&candidate, source) == Some(target) {
            shape(&candidate)
        } else {
            shape(full)
        }
    }

    /// Plan a rename of a file or folder: which files move and which links must be rewritten so
    /// that every link keeps pointing at the same file. The index is updated to the new paths;
    /// the caller performs the moves on disk, applies the edits and re-indexes the edited files.
    pub fn plan_rename(&mut self, old: &str, new: &str) -> RenamePlan {
        let old = path::normalize(old);
        let new = path::normalize(new);
        let mut plan = RenamePlan::default();
        if old == new || old.is_empty() || new.is_empty() {
            return plan;
        }
        let old_lower = old.to_lowercase();
        let prefix = format!("{old_lower}/");
        let mut moved: Vec<FileId> = Vec::new();
        for (id, f) in self.iter() {
            let pl = f.path.to_lowercase();
            if pl == old_lower {
                plan.moves.push((f.path.to_string(), new.clone()));
                moved.push(id);
            } else if pl.starts_with(&prefix) {
                plan.moves.push((f.path.to_string(), format!("{new}/{}", &f.path[prefix.len()..])));
                moved.push(id);
            }
        }
        if plan.moves.is_empty() {
            return plan;
        }
        // Sources whose links may change: everything linking to a moved file, plus the moved files.
        let graph = self.graph();
        let mut sources: Vec<FileId> = moved.clone();
        for &m in &moved {
            sources.extend(&graph.back[m as usize]);
        }
        // A moved file can also shadow the target of an unrelated link (a new `Note.md` at the
        // vault root captures `[[Note]]` written elsewhere). Include every file that has a link
        // whose last path segment matches a new file name, so those links are pinned too.
        let mut new_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (_, n) in &plan.moves {
            let name = path::file_name(n).to_lowercase();
            new_names.insert(path::strip_md(&name).to_string());
            new_names.insert(name);
        }
        for (id, f) in self.iter() {
            let mut hit = false;
            f.link_targets(|t| {
                hit |= new_names.contains(&path::file_name(&t.replace('\\', "/")).to_lowercase());
            });
            if hit {
                sources.push(id);
            }
        }
        sources.sort_unstable();
        sources.dedup();
        let metas: Vec<Option<Arc<NoteMeta>>> =
            sources.iter().map(|&s| self.entry(s).and_then(FileEntry::meta)).collect();
        let before: Vec<Vec<Option<FileId>>> = sources
            .iter()
            .zip(&metas)
            .map(|(&s, m)| match (self.entry(s), m) {
                (Some(f), Some(m)) => m.links.iter().map(|l| self.resolve(&l.target, &f.path)).collect(),
                _ => Vec::new(),
            })
            .collect();
        for (o, n) in &plan.moves {
            self.move_entry(o, n);
        }
        let format = self.link_format;
        for ((&s, pre), meta) in sources.iter().zip(&before).zip(&metas) {
            let Some(f) = self.entry(s) else { continue };
            let Some(meta) = meta else { continue };
            let mut edits = Vec::new();
            for (l, &pre) in meta.links.iter().zip(pre) {
                let Some(target) = pre else { continue };
                if self.resolve(&l.target, &f.path) == Some(target) {
                    continue;
                }
                edits.push(TextEdit {
                    start: l.target_start as usize,
                    end: l.target_end as usize,
                    text: self.rewritten_target(l, target, &f.path, format),
                });
            }
            if !edits.is_empty() {
                plan.edits.push(FileEdits { path: f.path.to_string(), edits });
            }
        }
        plan
    }

    fn rewritten_target(&self, l: &Link, target: FileId, source: &str, format: LinkFormat) -> String {
        let markdown = l.kind == LinkKind::Markdown;
        // Keep the style the link was written in where it is unambiguous.
        let written_relative = l.target.starts_with("./") || l.target.starts_with("../");
        let written_path = l.target.contains('/');
        let format = if written_relative {
            LinkFormat::Relative
        } else if written_path && format == LinkFormat::Shortest {
            LinkFormat::Absolute
        } else {
            format
        };
        let mut text = self.link_text(target, source, format, markdown);
        let keeps_md = l.target.len() > 3 && l.target[l.target.len() - 3..].eq_ignore_ascii_case(".md");
        if !markdown && keeps_md && self.entry(target).is_some_and(|f| f.kind == FileKind::Markdown) {
            text.push_str(".md");
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::extract;

    fn vault(files: &[(&str, &str)]) -> Vault {
        let mut v = Vault::new();
        for (p, text) in files {
            let meta = (path::kind_of(p) == FileKind::Markdown).then(|| extract(text));
            v.upsert(p, 0, text.len() as u64, meta);
        }
        v
    }

    fn res<'a>(v: &'a Vault, t: &str, from: &str) -> Option<&'a str> {
        v.resolve(t, from).and_then(|id| v.entry(id)).map(|f| &*f.path)
    }

    #[test]
    fn resolution_rules() {
        let v = vault(&[
            ("Note.md", ""),
            ("a/Note.md", ""),
            ("a/b/Deep.md", ""),
            ("a/Other.md", ""),
            ("z/Deep.md", ""),
            ("img/Pic.PNG", ""),
            ("Note.v2.md", ""),
        ]);
        assert_eq!(res(&v, "Note", "x.md"), Some("Note.md"));
        assert_eq!(res(&v, "note", "x.md"), Some("Note.md"), "case-insensitive");
        assert_eq!(res(&v, "Note.md", "x.md"), Some("Note.md"));
        assert_eq!(res(&v, "Note", "a/Other.md"), Some("Note.md"), "exact vault path wins");
        assert_eq!(res(&v, "a/Note", "x.md"), Some("a/Note.md"));
        assert_eq!(res(&v, "Deep", "x.md"), Some("z/Deep.md"), "shallowest");
        assert_eq!(res(&v, "Deep", "a/b/X.md"), Some("a/b/Deep.md"), "same folder first");
        assert_eq!(res(&v, "b/Deep", "x.md"), Some("a/b/Deep.md"), "partial path suffix");
        assert_eq!(res(&v, "pic.png", "x.md"), Some("img/Pic.PNG"));
        assert_eq!(res(&v, "Note.v2", "x.md"), Some("Note.v2.md"));
        assert_eq!(res(&v, "../Other", "a/b/Deep.md"), Some("a/Other.md"));
        assert_eq!(res(&v, "./Nope", "a/b/Deep.md"), None);
        assert_eq!(res(&v, "Missing", "x.md"), None);
        assert_eq!(res(&v, "", "a/Note.md"), Some("a/Note.md"), "empty target is the note itself");
        assert_eq!(res(&v, "ote", "x.md"), None, "no partial name match");
    }

    #[test]
    fn graph_and_backlinks() {
        let mut v =
            vault(&[("A.md", "[[B]] [[B]] [[Missing]] ![[p.png]]"), ("B.md", "[[A]] [[B]]"), ("p.png", "")]);
        let (a, b, p) = (v.id("A.md").unwrap(), v.id("B.md").unwrap(), v.id("p.png").unwrap());
        let g = v.graph();
        assert_eq!(g.out[a as usize], vec![(b, 2), (p, 1)]);
        assert_eq!(g.unresolved[a as usize], vec!["Missing".into()]);
        assert_eq!(g.back[b as usize], vec![a], "self links are not backlinks");
        assert_eq!(g.back[a as usize], vec![b]);
        assert_eq!(g.back[p as usize], vec![a]);
    }

    #[test]
    fn link_text_formats() {
        let v = vault(&[("a/Note.md", ""), ("b/Note.md", ""), ("b/Solo File.md", ""), ("b/c/x.png", "")]);
        let solo = v.id("b/Solo File.md").unwrap();
        let dup = v.id("b/Note.md").unwrap();
        let img = v.id("b/c/x.png").unwrap();
        assert_eq!(v.link_text(solo, "a/Note.md", LinkFormat::Shortest, false), "Solo File");
        assert_eq!(v.link_text(solo, "a/Note.md", LinkFormat::Shortest, true), "Solo%20File.md");
        assert_eq!(
            v.link_text(dup, "a/Note.md", LinkFormat::Shortest, false),
            "b/Note",
            "ambiguous name needs a path"
        );
        assert_eq!(
            v.link_text(dup, "b/Solo File.md", LinkFormat::Shortest, false),
            "Note",
            "same folder disambiguates"
        );
        assert_eq!(v.link_text(solo, "a/Note.md", LinkFormat::Relative, false), "../b/Solo File");
        assert_eq!(v.link_text(solo, "a/Note.md", LinkFormat::Absolute, false), "b/Solo File");
        assert_eq!(v.link_text(img, "a/Note.md", LinkFormat::Shortest, false), "x.png");
    }

    #[test]
    fn rename_rewrites_links_and_keeps_style() {
        let src = "[[Old]] [[Old#H|alias]] ![[Old#^b]] [md](Old.md#H) [[dir/Old.md]] [[Other]]";
        let mut v = vault(&[("Hub.md", src), ("dir/Old.md", "self [[Old#top]] [[Other]]"), ("Other.md", "")]);
        let plan = v.plan_rename("dir/Old.md", "dir/New Name.md");
        assert_eq!(plan.moves, vec![("dir/Old.md".to_string(), "dir/New Name.md".to_string())]);
        let hub = plan.edits.iter().find(|e| e.path == "Hub.md").unwrap();
        assert_eq!(
            apply_edits(src, &hub.edits),
            "[[New Name]] [[New Name#H|alias]] ![[New Name#^b]] [md](New%20Name.md#H) [[dir/New Name.md]] [[Other]]"
        );
        let own = plan.edits.iter().find(|e| e.path == "dir/New Name.md").unwrap();
        assert_eq!(apply_edits("self [[Old#top]] [[Other]]", &own.edits), "self [[New Name#top]] [[Other]]");
    }

    #[test]
    fn folder_rename_and_relative_links() {
        let a = "[[../x/B]] [b](../x/B.md) [[B]]";
        let mut v = vault(&[("p/A.md", a), ("x/B.md", "[[A]] [up](../p/A.md)"), ("x/sub/C.md", "")]);
        let plan = v.plan_rename("x", "y/z");
        assert_eq!(plan.moves.len(), 2);
        let ea = plan.edits.iter().find(|e| e.path == "p/A.md").unwrap();
        assert_eq!(apply_edits(a, &ea.edits), "[[../y/z/B]] [b](../y/z/B.md) [[B]]");
        let eb = plan.edits.iter().find(|e| e.path == "y/z/B.md").unwrap();
        assert_eq!(apply_edits("[[A]] [up](../p/A.md)", &eb.edits), "[[A]] [up](../../p/A.md)");
        assert!(v.id("y/z/sub/C.md").is_some() && v.id("x/B.md").is_none());
    }

    #[test]
    fn move_without_name_change_leaves_bare_links_alone() {
        let mut v = vault(&[("Hub.md", "[[Note]]"), ("Note.md", "")]);
        let plan = v.plan_rename("Note.md", "archive/Note.md");
        assert!(plan.edits.is_empty(), "[[Note]] still resolves by name");
    }
}
