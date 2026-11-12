use std::path::Path;

/// Normalise a vault-relative path and reject anything that could leave the vault or touch
/// app-private folders (`.obsidian`, `.walnut`, `.git`, `.trash`: every dot-component).
pub fn safe_rel(p: &str) -> Result<String, String> {
    if p.contains('\0') || p.contains('\\') || p.starts_with('/') || p.contains(':') {
        return Err(format!("invalid path: {p:?}"));
    }
    let mut parts = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            s if s.starts_with('.') => return Err(format!("path not allowed: {p:?}")),
            s => parts.push(s),
        }
    }
    if parts.is_empty() {
        return Err("empty path".into());
    }
    Ok(parts.join("/"))
}

/// `safe_rel` plus a check that symlinks do not lead out of the vault.
pub fn guard_path(root: &Path, p: &str) -> Result<String, String> {
    let rel = safe_rel(p)?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let mut probe = root.join(&rel);
    loop {
        match probe.canonicalize() {
            Ok(real) => {
                return if real.starts_with(&root) {
                    Ok(rel)
                } else {
                    Err(format!("path escapes the vault: {p:?}"))
                };
            }
            Err(_) => {
                if !probe.pop() || probe.as_os_str().is_empty() {
                    return Ok(rel);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal_and_private_dirs() {
        for bad in [
            "../x.md",
            "a/../../x",
            "/etc/passwd",
            "a\\b",
            ".obsidian/app.json",
            "a/.git/x",
            "C:/x",
            "",
            "a\0b",
        ] {
            assert!(safe_rel(bad).is_err(), "{bad}");
        }
        assert_eq!(safe_rel("a//b/./c.md").unwrap(), "a/b/c.md");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_rejected() {
        let outside = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), vault.path().join("link")).unwrap();
        assert!(guard_path(vault.path(), "link/x.md").is_err());
        assert!(guard_path(vault.path(), "new/x.md").is_ok());
    }
}
