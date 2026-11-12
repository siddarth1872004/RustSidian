//! Disk primitives: atomic writes, trash, file-recovery snapshots.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// (mtime in ns since the epoch, size). Used to detect external changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Stamp {
    pub mtime: i64,
    pub size: u64,
}

pub fn stamp_of(meta: &fs::Metadata) -> Stamp {
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos().min(i64::MAX as u128) as i64);
    Stamp { mtime, size: meta.len() }
}

pub fn stamp(path: &Path) -> io::Result<Stamp> {
    Ok(stamp_of(&fs::metadata(path)?))
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Write `data` to `path` so that a crash leaves either the old or the new content, never a
/// mix: write a temp file in the same directory, fsync it, rename over the target, fsync the
/// directory.
pub fn atomic_write(path: &Path, data: &[u8]) -> io::Result<()> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!(
        ".{name}.walnut-tmp-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut f = File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);
        // Keep the permissions of the file being replaced.
        if let Ok(meta) = fs::metadata(path) {
            let _ = fs::set_permissions(&tmp, meta.permissions());
        }
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
        return result;
    }
    #[cfg(unix)]
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TrashMode {
    /// `.trash/` inside the vault (Obsidian's "Obsidian trash").
    #[default]
    Vault,
    /// The desktop trash (XDG trash on Linux). Falls back to the vault trash where unsupported.
    System,
    Permanent,
}

fn unique_dest(dir: &Path, name: &str) -> PathBuf {
    let mut dest = dir.join(name);
    let mut n = 1;
    while dest.exists() {
        let (stem, ext) = match name.rfind('.') {
            Some(i) if i > 0 => (&name[..i], &name[i..]),
            _ => (name, ""),
        };
        n += 1;
        dest = dir.join(format!("{stem} {n}{ext}"));
    }
    dest
}

fn move_path(from: &Path, to: &Path) -> io::Result<()> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    // Different filesystem: copy then delete.
    if from.is_dir() {
        fs::create_dir_all(to)?;
        for e in fs::read_dir(from)? {
            let e = e?;
            move_path(&e.path(), &to.join(e.file_name()))?;
        }
        fs::remove_dir(from)
    } else {
        fs::copy(from, to)?;
        fs::remove_file(from)
    }
}

/// Move a file or folder to the trash. Returns where it went (`None` for permanent deletion).
pub fn trash(vault_root: &Path, abs: &Path, mode: TrashMode) -> io::Result<Option<PathBuf>> {
    let name = abs.file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();
    match mode {
        TrashMode::Permanent => {
            if abs.is_dir() {
                fs::remove_dir_all(abs)?;
            } else {
                fs::remove_file(abs)?;
            }
            Ok(None)
        }
        TrashMode::System => {
            if let Some(dest) = system_trash(abs, &name)? {
                return Ok(Some(dest));
            }
            trash(vault_root, abs, TrashMode::Vault)
        }
        TrashMode::Vault => {
            let dir = vault_root.join(".trash");
            fs::create_dir_all(&dir)?;
            let dest = unique_dest(&dir, &name);
            move_path(abs, &dest)?;
            Ok(Some(dest))
        }
    }
}

/// XDG trash specification: `$XDG_DATA_HOME/Trash/{files,info}`.
#[cfg(all(unix, not(target_os = "macos")))]
fn system_trash(abs: &Path, name: &str) -> io::Result<Option<PathBuf>> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    let Some(data) = data else { return Ok(None) };
    let files = data.join("Trash/files");
    let info = data.join("Trash/info");
    fs::create_dir_all(&files)?;
    fs::create_dir_all(&info)?;
    let dest = unique_dest(&files, name);
    let dest_name = dest.file_name().and_then(|n| n.to_str()).unwrap_or(name).to_string();
    let secs = now_ms() / 1000;
    let (y, mo, d, h, mi, s) = civil(secs);
    let encoded: String = abs
        .to_string_lossy()
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    let record =
        format!("[Trash Info]\nPath={encoded}\nDeletionDate={y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}\n");
    atomic_write(&info.join(format!("{dest_name}.trashinfo")), record.as_bytes())?;
    move_path(abs, &dest)?;
    Ok(Some(dest))
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn system_trash(_abs: &Path, _name: &str) -> io::Result<Option<PathBuf>> {
    Ok(None)
}

/// Seconds since the epoch to a UTC civil date-time.
pub fn civil(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, (rem / 3600) as u32, (rem % 3600 / 60) as u32, (rem % 60) as u32)
}

pub fn fold64(data: &[u8]) -> u64 {
    let mut h: u64 = 0x9e37_79b9_7f4a_7c15 ^ data.len() as u64;
    let (chunks, rest) = data.as_chunks::<8>();
    for c in chunks {
        h = (h ^ u64::from_le_bytes(*c)).wrapping_mul(0xff51_afd7_ed55_8ccd).rotate_left(29);
    }
    for &b in rest {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    h ^ (h >> 32)
}

/// File recovery: periodic snapshots of note content under `.walnut/recovery/`.
pub struct Recovery {
    dir: PathBuf,
    /// Minimum time between two snapshots of the same file.
    pub interval_ms: u64,
    /// Snapshots older than this are pruned.
    pub keep_ms: u64,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Snapshot {
    pub time_ms: u64,
    pub path: PathBuf,
}

impl Recovery {
    pub fn new(vault_root: &Path) -> Recovery {
        Recovery {
            dir: vault_root.join(".walnut/recovery"),
            interval_ms: 5 * 60_000,
            keep_ms: 7 * 86_400_000,
        }
    }

    fn dir_for(&self, rel: &str) -> PathBuf {
        self.dir.join(format!("{:016x}", fold64(rel.to_lowercase().as_bytes())))
    }

    pub fn list(&self, rel: &str) -> Vec<Snapshot> {
        let mut out: Vec<Snapshot> = fs::read_dir(self.dir_for(rel))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name();
                let t = name.to_str()?.strip_suffix(".snap")?.parse().ok()?;
                Some(Snapshot { time_ms: t, path: e.path() })
            })
            .collect();
        out.sort_by_key(|s| s.time_ms);
        out
    }

    /// Store a snapshot if the last one is older than the interval and the content differs.
    pub fn snapshot(&self, rel: &str, text: &str, now_ms: u64) -> io::Result<bool> {
        let existing = self.list(rel);
        if let Some(last) = existing.last() {
            if now_ms.saturating_sub(last.time_ms) < self.interval_ms {
                return Ok(false);
            }
            if fs::read(&last.path).is_ok_and(|b| b == text.as_bytes()) {
                return Ok(false);
            }
        }
        let dir = self.dir_for(rel);
        fs::create_dir_all(&dir)?;
        atomic_write(&dir.join("path.txt"), rel.as_bytes())?;
        atomic_write(&dir.join(format!("{now_ms}.snap")), text.as_bytes())?;
        for old in existing.iter().filter(|s| now_ms.saturating_sub(s.time_ms) > self.keep_ms) {
            let _ = fs::remove_file(&old.path);
        }
        Ok(true)
    }

    pub fn read(&self, snap: &Snapshot) -> io::Result<String> {
        fs::read_to_string(&snap.path)
    }

    pub fn clear(&self) -> io::Result<()> {
        if self.dir.exists() {
            fs::remove_dir_all(&self.dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub/a.md");
        atomic_write(&p, b"one").unwrap();
        atomic_write(&p, b"two").unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "two");
        let names: Vec<_> =
            fs::read_dir(p.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1, "{names:?}");
    }

    #[test]
    fn vault_trash_keeps_duplicates() {
        let d = tempfile::tempdir().unwrap();
        for _ in 0..2 {
            let p = d.path().join("n.md");
            fs::write(&p, "x").unwrap();
            trash(d.path(), &p, TrashMode::Vault).unwrap();
            assert!(!p.exists());
        }
        assert!(d.path().join(".trash/n.md").exists() && d.path().join(".trash/n 2.md").exists());
        let p = d.path().join("gone.md");
        fs::write(&p, "x").unwrap();
        assert_eq!(trash(d.path(), &p, TrashMode::Permanent).unwrap(), None);
        assert!(!p.exists());
    }

    #[test]
    fn recovery_interval_dedup_and_prune() {
        let d = tempfile::tempdir().unwrap();
        let mut r = Recovery::new(d.path());
        r.interval_ms = 1000;
        r.keep_ms = 10_000;
        assert!(r.snapshot("a.md", "v1", 1_000).unwrap());
        assert!(!r.snapshot("a.md", "v2", 1_500).unwrap(), "inside the interval");
        assert!(!r.snapshot("a.md", "v1", 3_000).unwrap(), "unchanged content");
        assert!(r.snapshot("a.md", "v2", 3_000).unwrap());
        assert!(r.snapshot("a.md", "v3", 20_000).unwrap());
        let list = r.list("a.md");
        assert_eq!(list.iter().map(|s| s.time_ms).collect::<Vec<_>>(), vec![20_000], "old snapshots pruned");
        assert_eq!(r.read(&list[0]).unwrap(), "v3");
        assert!(r.list("b.md").is_empty());
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(1_709_210_096), (2024, 2, 29, 12, 34, 56));
    }
}
