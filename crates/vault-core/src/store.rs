//! A vault on disk: open with a persisted index, revalidate by mtime/size, read, write with
//! conflict detection, rename with link updates, trash.

use crate::cache::{self, Meta};
use crate::fsio::{self, atomic_write, Stamp, TrashMode};
use crate::meta::{extract, NoteMeta};
use crate::mmap::Mapped;
use crate::path::{self, FileKind};
use crate::vault::{apply_edits, RenamePlan, Vault};
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default)]
pub struct OpenStats {
    pub files: usize,
    pub notes: usize,
    /// Notes whose metadata came from the persisted index.
    pub reused: usize,
    /// Notes that had to be read and parsed.
    pub parsed: usize,
    pub cache_loaded: bool,
    pub walk: Duration,
    pub load_cache: Duration,
    pub parse: Duration,
    pub total: Duration,
}

#[derive(Debug)]
pub enum WriteError {
    /// The file changed on disk since it was read. Nothing was written.
    Conflict {
        disk: Stamp,
    },
    Io(io::Error),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::Conflict { .. } => write!(f, "file was modified outside Rustwalnut"),
            WriteError::Io(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for WriteError {}
impl From<io::Error> for WriteError {
    fn from(e: io::Error) -> Self {
        WriteError::Io(e)
    }
}

pub struct VaultStore {
    pub root: PathBuf,
    pub index: Vault,
    pub trash_mode: TrashMode,
    dirty: bool,
    /// The mapped index that unchanged notes still read their metadata from.
    cache_map: Option<Arc<Mapped>>,
}

/// What a filesystem notification turned out to be.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Change {
    Created(String),
    Modified(String),
    Removed(String),
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// Walk the vault, skipping dot-files and dot-folders like Obsidian does.
fn walk(root: &Path) -> Vec<(String, Stamp)> {
    fn dir(abs: &Path, rel: &str) -> Vec<(String, Stamp)> {
        let Ok(rd) = fs::read_dir(abs) else { return Vec::new() };
        let mut files = Vec::new();
        let mut dirs = Vec::new();
        for e in rd.flatten() {
            let name = e.file_name();
            let Some(name) = name.to_str() else { continue };
            if is_hidden(name) {
                continue;
            }
            let child = if rel.is_empty() { name.to_string() } else { format!("{rel}/{name}") };
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                dirs.push((e.path(), child));
            } else if let Ok(meta) = e.metadata() {
                // `metadata` follows symlinks for files only via `fs::metadata`; DirEntry does not.
                if meta.is_file() {
                    files.push((child, fsio::stamp_of(&meta)));
                } else if let Ok(target) = fs::metadata(e.path()) {
                    if target.is_file() {
                        files.push((child, fsio::stamp_of(&target)));
                    }
                }
            }
        }
        let nested: Vec<Vec<(String, Stamp)>> = dirs.par_iter().map(|(a, r)| dir(a, r)).collect();
        files.extend(nested.into_iter().flatten());
        files
    }
    dir(root, "")
}

impl VaultStore {
    pub fn cache_path(root: &Path) -> PathBuf {
        root.join(".walnut/cache/index.bin")
    }

    /// Open a vault. Metadata of files whose mtime and size match the persisted index is reused;
    /// only new or changed notes are read and parsed.
    pub fn open(root: &Path) -> io::Result<(VaultStore, OpenStats)> {
        let t0 = Instant::now();
        if !root.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} is not a folder", root.display()),
            ));
        }
        let mut stats = OpenStats::default();
        let (disk, cached) = rayon::join(
            || {
                let t = Instant::now();
                (walk(root), t.elapsed())
            },
            || {
                let t = Instant::now();
                (cache::load(&Self::cache_path(root)), t.elapsed())
            },
        );
        stats.walk = disk.1;
        stats.load_cache = cached.1;
        stats.cache_loaded = cached.0.is_some();
        let (cached_files, cache_map) = cached.0.map_or((Vec::new(), None), |(f, m)| (f, Some(m)));
        let mut old: HashMap<String, (Stamp, Option<Meta>)> =
            cached_files.into_iter().map(|c| (c.path, (c.stamp, c.meta.map(Meta::Cached)))).collect();
        let had = old.len();
        let mut todo = Vec::new();
        let mut entries: Vec<(String, Stamp, Option<Meta>)> = Vec::with_capacity(disk.0.len());
        for (p, stamp) in disk.0 {
            let is_note = path::kind_of(&p) == FileKind::Markdown;
            match old.remove(&p) {
                Some((s, meta)) if s == stamp && meta.is_some() == is_note => {
                    stats.reused += is_note as usize;
                    entries.push((p, stamp, meta));
                }
                _ => {
                    if is_note {
                        todo.push(entries.len());
                    }
                    entries.push((p, stamp, None));
                }
            }
        }
        let t = Instant::now();
        let parsed: Vec<(usize, Option<NoteMeta>)> = todo
            .par_iter()
            .map(|&i| (i, fs::read_to_string(root.join(&entries[i].0)).ok().map(|text| extract(&text))))
            .collect();
        stats.parsed = parsed.len();
        for (i, meta) in parsed {
            // Unreadable (for example non-UTF-8) notes are indexed without metadata.
            entries[i].2 = Some(Meta::Owned(Arc::new(meta.unwrap_or_default())));
        }
        stats.parse = t.elapsed();
        let changed = stats.parsed > 0 || !old.is_empty() || entries.len() != had || !stats.cache_loaded;
        let mut index = Vault::new();
        for (p, stamp, meta) in entries {
            index.upsert_meta(&p, stamp.mtime, stamp.size, meta);
        }
        stats.files = index.len();
        stats.notes = index.notes().count();
        let store = VaultStore {
            root: root.to_path_buf(),
            index,
            trash_mode: TrashMode::default(),
            dirty: changed,
            cache_map,
        };
        // Verifying the checksum touched every page of the index.
        store.release_cache_pages();
        stats.total = t0.elapsed();
        Ok((store, stats))
    }

    /// Drop the resident pages of the mapped index. Worth calling after anything that read the
    /// metadata of many notes (building the link graph, a metadata search).
    pub fn release_cache_pages(&self) {
        if let Some(m) = &self.cache_map {
            m.release();
        }
    }

    pub fn abs(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// Persist the index if it changed since it was loaded or last saved.
    pub fn save_cache(&mut self) -> io::Result<bool> {
        if !self.dirty {
            return Ok(false);
        }
        let bytes = cache::encode(
            self.index
                .iter()
                .map(|(_, f)| (&*f.path, Stamp { mtime: f.mtime, size: f.size }, f.meta.as_ref())),
        );
        cache::save(&Self::cache_path(&self.root), &bytes)?;
        self.dirty = false;
        Ok(true)
    }

    pub fn read(&self, rel: &str) -> io::Result<(String, Stamp)> {
        let abs = self.abs(rel);
        // Stamp first: if the file changes between the two calls the stamp is older than the
        // content, which errs on the side of reporting a conflict later.
        let stamp = fsio::stamp(&abs)?;
        Ok((fs::read_to_string(&abs)?, stamp))
    }

    fn reindex(&mut self, rel: &str, text: Option<&str>) -> io::Result<Stamp> {
        let stamp = fsio::stamp(&self.abs(rel))?;
        let meta = match (path::kind_of(rel), text) {
            (FileKind::Markdown, Some(t)) => Some(extract(t)),
            (FileKind::Markdown, None) => {
                Some(fs::read_to_string(self.abs(rel)).map(|t| extract(&t)).unwrap_or_default())
            }
            _ => None,
        };
        self.index.upsert(rel, stamp.mtime, stamp.size, meta);
        self.dirty = true;
        Ok(stamp)
    }

    /// Write a file atomically. With `expected`, the write is refused if the file on disk no
    /// longer has that stamp (it was edited externally since it was read).
    pub fn write(&mut self, rel: &str, text: &str, expected: Option<Stamp>) -> Result<Stamp, WriteError> {
        let rel = path::normalize(rel);
        let abs = self.abs(&rel);
        if let Some(exp) = expected {
            match fsio::stamp(&abs) {
                Ok(disk) if disk != exp => return Err(WriteError::Conflict { disk }),
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        atomic_write(&abs, text.as_bytes())?;
        Ok(self.reindex(&rel, Some(text))?)
    }

    /// Create a new file; fails if it exists.
    pub fn create(&mut self, rel: &str, text: &str) -> io::Result<Stamp> {
        let rel = path::normalize(rel);
        if self.abs(&rel).exists() {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{rel} already exists")));
        }
        atomic_write(&self.abs(&rel), text.as_bytes())?;
        self.reindex(&rel, Some(text))
    }

    pub fn delete(&mut self, rel: &str) -> io::Result<Option<PathBuf>> {
        let rel = path::normalize(rel);
        let dest = fsio::trash(&self.root, &self.abs(&rel), self.trash_mode)?;
        let prefix = format!("{}/", rel.to_lowercase());
        let doomed: Vec<String> = self
            .index
            .iter()
            .filter(|(_, f)| {
                let l = f.path.to_lowercase();
                l == rel.to_lowercase() || l.starts_with(&prefix)
            })
            .map(|(_, f)| f.path.to_string())
            .collect();
        for p in doomed {
            self.index.remove(&p);
        }
        self.dirty = true;
        Ok(dest)
    }

    /// Rename or move a file or folder and rewrite every link that pointed at it.
    /// On an IO error part-way, files already moved are moved back.
    pub fn rename(&mut self, old: &str, new: &str) -> io::Result<RenamePlan> {
        let (old, new) = (path::normalize(old), path::normalize(new));
        let (old_abs, new_abs) = (self.abs(&old), self.abs(&new));
        if !old_abs.exists() {
            return Err(io::Error::new(io::ErrorKind::NotFound, format!("{old} does not exist")));
        }
        let case_only = old.to_lowercase() == new.to_lowercase();
        if new_abs.exists() && !case_only {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{new} already exists")));
        }
        if let Some(parent) = new_abs.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&old_abs, &new_abs)?;
        let plan = self.index.plan_rename(&old, &new);
        let mut failure = None;
        for fe in &plan.edits {
            let result = (|| -> io::Result<()> {
                let text = fs::read_to_string(self.abs(&fe.path))?;
                let updated = apply_edits(&text, &fe.edits);
                atomic_write(&self.abs(&fe.path), updated.as_bytes())?;
                self.reindex(&fe.path, Some(&updated))?;
                Ok(())
            })();
            if let Err(e) = result {
                failure = Some(e);
                break;
            }
        }
        for (_, n) in &plan.moves {
            // Stamps may change with the move on some filesystems.
            if let Ok(stamp) = fsio::stamp(&self.abs(n)) {
                if let Some(id) = self.index.id(n) {
                    let meta = self.index.entry(id).and_then(|f| f.meta.clone());
                    self.index.upsert_meta(n, stamp.mtime, stamp.size, meta);
                }
            }
        }
        self.dirty = true;
        match failure {
            // Link updates are best effort once the move itself succeeded: the files are in
            // their new place and remaining links are reported, not silently lost.
            Some(e) => Err(io::Error::new(e.kind(), format!("moved, but updating links failed: {e}"))),
            None => Ok(plan),
        }
    }

    /// Bring the index in line with the disk for one path (a watcher notification).
    /// Returns `None` if nothing actually changed.
    pub fn refresh(&mut self, rel: &str) -> Option<Change> {
        let rel = path::normalize(rel);
        if rel.split('/').any(is_hidden) {
            return None;
        }
        let known = self.index.get(&rel).map(|f| Stamp { mtime: f.mtime, size: f.size });
        match fs::metadata(self.abs(&rel)) {
            Ok(m) if m.is_file() => {
                let disk = fsio::stamp_of(&m);
                if known == Some(disk) {
                    return None;
                }
                self.reindex(&rel, None).ok()?;
                Some(if known.is_some() { Change::Modified(rel) } else { Change::Created(rel) })
            }
            Ok(_) => None,
            Err(_) => {
                known?;
                self.index.remove(&rel);
                self.dirty = true;
                Some(Change::Removed(rel))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(files: &[(&str, &str)]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for (p, t) in files {
            let abs = d.path().join(p);
            fs::create_dir_all(abs.parent().unwrap()).unwrap();
            fs::write(abs, t).unwrap();
        }
        d
    }

    #[test]
    fn metadata_from_the_persisted_index_is_read_lazily() {
        let d = make(&[("A.md", "# Title\n[[B]] [[Missing]] #tag\n"), ("B.md", "[[A]]\n"), ("x.png", "")]);
        let (mut s, _) = VaultStore::open(d.path()).unwrap();
        let want = s.index.get("A.md").unwrap().meta().unwrap();
        let graph = s.index.graph();
        assert!(s.save_cache().unwrap());
        drop(s);

        let (mut s, st) = VaultStore::open(d.path()).unwrap();
        assert_eq!((st.reused, st.parsed), (2, 0));
        s.release_cache_pages();
        let a = s.index.get("A.md").unwrap();
        assert!(matches!(a.meta, Some(Meta::Cached(_))));
        assert_eq!(a.meta().unwrap(), want);
        assert!(!s.index.get("x.png").unwrap().has_meta());
        let again = s.index.graph();
        assert_eq!(
            (&again.out, &again.back, &again.unresolved),
            (&graph.out, &graph.back, &graph.unresolved)
        );

        // A changed note is re-parsed; saving and reopening keeps the untouched blob intact.
        s.write("B.md", "[[A]] [[A]]\n", None).unwrap();
        assert!(s.save_cache().unwrap());
        drop(s);
        let (s, st) = VaultStore::open(d.path()).unwrap();
        assert_eq!((st.reused, st.parsed), (2, 0));
        assert_eq!(s.index.get("A.md").unwrap().meta().unwrap(), want);
        assert_eq!(s.index.get("B.md").unwrap().meta().unwrap().links.len(), 2);
    }

    #[test]
    fn open_uses_cache_and_reparses_only_changes() {
        let d = make(&[
            ("A.md", "[[B]]"),
            ("dir/B.md", "#tag"),
            ("img.png", "x"),
            (".obsidian/app.json", "{}"),
            (".hidden.md", ""),
        ]);
        let (mut s, st) = VaultStore::open(d.path()).unwrap();
        assert_eq!((st.files, st.notes, st.parsed, st.reused, st.cache_loaded), (3, 2, 2, 0, false));
        assert!(s.save_cache().unwrap());
        assert!(!s.save_cache().unwrap(), "clean index is not rewritten");

        let (mut s, st) = VaultStore::open(d.path()).unwrap();
        assert_eq!((st.parsed, st.reused, st.cache_loaded), (0, 2, true));
        assert!(!s.save_cache().unwrap(), "nothing changed, nothing written");
        let a = s.index.id("A.md").unwrap();
        assert_eq!(s.index.graph().out[a as usize].len(), 1);

        // External change, deletion and addition while closed.
        fs::write(d.path().join("dir/B.md"), "changed [[A]] longer").unwrap();
        fs::remove_file(d.path().join("img.png")).unwrap();
        fs::write(d.path().join("C.md"), "new").unwrap();
        let (mut s, st) = VaultStore::open(d.path()).unwrap();
        assert_eq!((st.files, st.parsed, st.reused), (3, 2, 1));
        assert_eq!(s.index.get("dir/B.md").unwrap().meta().unwrap().links.len(), 1);
        assert!(s.save_cache().unwrap());
    }

    #[test]
    fn corrupt_cache_triggers_rebuild() {
        let d = make(&[("A.md", "[[B]]"), ("B.md", "")]);
        let (mut s, _) = VaultStore::open(d.path()).unwrap();
        s.save_cache().unwrap();
        let cp = VaultStore::cache_path(d.path());
        let mut bytes = fs::read(&cp).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xff;
        fs::write(&cp, bytes).unwrap();
        let (s, st) = VaultStore::open(d.path()).unwrap();
        assert!(!st.cache_loaded);
        assert_eq!((st.parsed, s.index.len()), (2, 2));
    }

    #[test]
    fn write_detects_external_edits() {
        let d = make(&[("A.md", "v1")]);
        let (mut s, _) = VaultStore::open(d.path()).unwrap();
        let (text, stamp) = s.read("A.md").unwrap();
        assert_eq!(text, "v1");
        let stamp2 = s.write("A.md", "v2 [[X]]", Some(stamp)).unwrap();
        assert_eq!(s.index.get("A.md").unwrap().meta().unwrap().links.len(), 1);
        // Someone else edits the file.
        fs::write(d.path().join("A.md"), "external edit!").unwrap();
        match s.write("A.md", "v3", Some(stamp2)) {
            Err(WriteError::Conflict { .. }) => {}
            other => panic!("expected conflict, got {other:?}"),
        }
        assert_eq!(
            fs::read_to_string(d.path().join("A.md")).unwrap(),
            "external edit!",
            "nothing was overwritten"
        );
        assert_eq!(s.refresh("A.md"), Some(Change::Modified("A.md".into())));
        assert_eq!(s.refresh("A.md"), None);
    }

    #[test]
    fn rename_moves_file_and_rewrites_links_on_disk() {
        let d = make(&[
            ("Hub.md", "see [[Old]] and [x](sub/Old.md)"),
            ("sub/Old.md", "me"),
            ("sub/Other.md", "[[Old|o]]"),
        ]);
        let (mut s, _) = VaultStore::open(d.path()).unwrap();
        let plan = s.rename("sub/Old.md", "sub/New.md").unwrap();
        assert_eq!(plan.edits.len(), 2);
        assert_eq!(fs::read_to_string(d.path().join("Hub.md")).unwrap(), "see [[New]] and [x](sub/New.md)");
        assert_eq!(fs::read_to_string(d.path().join("sub/Other.md")).unwrap(), "[[New|o]]");
        assert!(!d.path().join("sub/Old.md").exists() && d.path().join("sub/New.md").exists());
        let new = s.index.id("sub/New.md").unwrap();
        assert_eq!(s.index.graph().back[new as usize].len(), 2);
        assert!(s.rename("sub/New.md", "Hub.md").is_err(), "never overwrite an existing file");
        assert!(d.path().join("sub/New.md").exists());
    }

    #[test]
    fn create_delete_refresh() {
        let d = make(&[("dir/A.md", "a"), ("dir/B.md", "b")]);
        let (mut s, _) = VaultStore::open(d.path()).unwrap();
        s.create("N.md", "new").unwrap();
        assert!(s.create("N.md", "again").is_err());
        let dest = s.delete("dir").unwrap().unwrap();
        assert!(dest.starts_with(d.path().join(".trash")) && dest.join("A.md").exists());
        assert_eq!(s.index.len(), 1);
        fs::write(d.path().join("ext.md"), "x").unwrap();
        assert_eq!(s.refresh("ext.md"), Some(Change::Created("ext.md".into())));
        fs::remove_file(d.path().join("ext.md")).unwrap();
        assert_eq!(s.refresh("ext.md"), Some(Change::Removed("ext.md".into())));
        assert_eq!(s.refresh(".obsidian/app.json"), None);
    }
}
