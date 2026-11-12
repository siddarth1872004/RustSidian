//! Filesystem watcher with debounce. Event-driven: the debounce thread blocks until the OS
//! reports a change, so an idle vault costs no CPU.

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

pub struct VaultWatcher {
    // Dropping the watcher closes the channel, which ends the debounce thread.
    _watcher: RecommendedWatcher,
}

impl VaultWatcher {
    /// Calls `on_change` with the vault-relative paths that changed, after `debounce` of quiet.
    /// Paths under dot-folders other than `.obsidian` are ignored.
    pub fn start(
        root: &Path,
        debounce: Duration,
        mut on_change: impl FnMut(Vec<String>) + Send + 'static,
    ) -> notify::Result<VaultWatcher> {
        let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
        let mut watcher = notify::recommended_watcher(tx)?;
        watcher.watch(root, RecursiveMode::Recursive)?;
        let root: PathBuf = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        std::thread::Builder::new()
            .name("vault-watch".into())
            .spawn(move || {
                let mut pending: BTreeSet<String> = BTreeSet::new();
                let add = |ev: notify::Result<notify::Event>, pending: &mut BTreeSet<String>| {
                    let Ok(ev) = ev else { return };
                    if matches!(ev.kind, notify::EventKind::Access(_)) {
                        return;
                    }
                    for p in ev.paths {
                        let Ok(rel) = p.strip_prefix(&root) else { continue };
                        let rel = rel.to_string_lossy().replace('\\', "/");
                        let hidden = rel.split('/').any(|s| s.starts_with('.') && s != ".obsidian");
                        if !rel.is_empty() && !hidden {
                            pending.insert(rel);
                        }
                    }
                };
                while let Ok(first) = rx.recv() {
                    add(first, &mut pending);
                    loop {
                        match rx.recv_timeout(debounce) {
                            Ok(ev) => add(ev, &mut pending),
                            Err(mpsc::RecvTimeoutError::Timeout) => break,
                            Err(mpsc::RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    if !pending.is_empty() {
                        on_change(std::mem::take(&mut pending).into_iter().collect());
                    }
                }
            })
            .map_err(notify::Error::io)?;
        Ok(VaultWatcher { _watcher: watcher })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn reports_debounced_relative_paths() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("sub")).unwrap();
        fs::create_dir_all(d.path().join(".walnut")).unwrap();
        let (tx, rx) = mpsc::channel();
        let _w = VaultWatcher::start(d.path(), Duration::from_millis(60), move |paths| {
            let _ = tx.send(paths);
        })
        .unwrap();
        fs::write(d.path().join("sub/a.md"), "1").unwrap();
        fs::write(d.path().join("sub/a.md"), "22").unwrap();
        fs::write(d.path().join(".walnut/cache"), "ignored").unwrap();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        while let Ok(batch) = rx.recv_timeout(Duration::from_secs(5)) {
            seen.extend(batch);
            if seen.contains("sub/a.md") {
                break;
            }
        }
        assert!(seen.contains("sub/a.md"), "{seen:?}");
        assert!(!seen.iter().any(|p| p.starts_with(".walnut")));
    }
}
