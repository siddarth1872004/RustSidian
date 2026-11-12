//! Vault-relative paths: forward slashes, no leading slash, no `.`/`..` segments.

/// Normalize a user- or OS-supplied relative path.
pub fn normalize(p: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for seg in p.split(['/', '\\']) {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    out.join("/")
}

pub fn parent(p: &str) -> &str {
    p.rfind('/').map_or("", |i| &p[..i])
}

pub fn file_name(p: &str) -> &str {
    p.rfind('/').map_or(p, |i| &p[i + 1..])
}

/// Extension without the dot, lowercased comparison is up to the caller.
pub fn extension(p: &str) -> &str {
    let name = file_name(p);
    match name.rfind('.') {
        Some(i) if i > 0 => &name[i + 1..],
        _ => "",
    }
}

/// File name without its extension.
pub fn stem(p: &str) -> &str {
    let name = file_name(p);
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

pub fn strip_md(p: &str) -> &str {
    if p.len() > 3 && p[p.len() - 3..].eq_ignore_ascii_case(".md") {
        &p[..p.len() - 3]
    } else {
        p
    }
}

pub fn join(dir: &str, rel: &str) -> String {
    if dir.is_empty() {
        normalize(rel)
    } else {
        normalize(&format!("{dir}/{rel}"))
    }
}

/// Path of `target` relative to the folder `from_dir`.
pub fn relative(from_dir: &str, target: &str) -> String {
    let a: Vec<&str> = from_dir.split('/').filter(|s| !s.is_empty()).collect();
    let b: Vec<&str> = target.split('/').filter(|s| !s.is_empty()).collect();
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let common = common.min(b.len().saturating_sub(1));
    let mut out: Vec<&str> = vec![".."; a.len() - common];
    out.extend(&b[common..]);
    out.join("/")
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileKind {
    Markdown,
    Canvas,
    Base,
    Attachment,
}

pub fn kind_of(p: &str) -> FileKind {
    let e = extension(p);
    if e.eq_ignore_ascii_case("md") {
        FileKind::Markdown
    } else if e.eq_ignore_ascii_case("canvas") {
        FileKind::Canvas
    } else if e.eq_ignore_ascii_case("base") {
        FileKind::Base
    } else {
        FileKind::Attachment
    }
}

/// How characters that are unsafe in file names are handled when a note is created or renamed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SanitizePolicy {
    /// Remove characters Obsidian disallows (`* " \ / < > : | ?`) plus link-breaking ones (`# ^ [ ]`).
    #[default]
    Strip,
    /// Replace them with `-`.
    Replace,
    /// Only remove what the operating systems reject; keep `# ^ [ ]`.
    OsOnly,
}

pub fn sanitize_file_name(name: &str, policy: SanitizePolicy) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        let os_bad = matches!(c, '*' | '"' | '\\' | '/' | '<' | '>' | ':' | '|' | '?') || c.is_control();
        let link_bad = matches!(c, '#' | '^' | '[' | ']');
        let bad = os_bad || (link_bad && policy != SanitizePolicy::OsOnly);
        if !bad {
            out.push(c);
        } else if policy == SanitizePolicy::Replace {
            out.push('-');
        }
    }
    let trimmed = out.trim().trim_end_matches('.').trim_start_matches('.').trim();
    if trimmed.is_empty() {
        "Untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() && s.is_char_boundary(i + 1) && s.is_char_boundary(i + 3) {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// Encode a path for use as a Markdown link destination.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '(' => out.push_str("%28"),
            ')' => out.push_str("%29"),
            '%' => out.push_str("%25"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_and_parts() {
        assert_eq!(normalize("./a//b/../c.md"), "a/c.md");
        assert_eq!(normalize("a\\b"), "a/b");
        assert_eq!(parent("a/b/c.md"), "a/b");
        assert_eq!(parent("c.md"), "");
        assert_eq!(file_name("a/b/c.md"), "c.md");
        assert_eq!(stem("a/b/c.v2.md"), "c.v2");
        assert_eq!(extension("a/.hidden"), "");
        assert_eq!(strip_md("a/Note.MD"), "a/Note");
        assert_eq!(join("a/b", "../c.md"), "a/c.md");
    }

    #[test]
    fn relative_paths() {
        assert_eq!(relative("a/b", "a/c/d.md"), "../c/d.md");
        assert_eq!(relative("", "a/d.md"), "a/d.md");
        assert_eq!(relative("a", "a/d.md"), "d.md");
        assert_eq!(relative("a/b", "d.md"), "../../d.md");
        assert_eq!(join("a/b", &relative("a/b", "x/y.md")), "x/y.md");
    }

    #[test]
    fn sanitize() {
        assert_eq!(sanitize_file_name("a/b: c? [x]#1", SanitizePolicy::Strip), "ab c x1");
        assert_eq!(sanitize_file_name("a:b", SanitizePolicy::Replace), "a-b");
        assert_eq!(sanitize_file_name("a#b:", SanitizePolicy::OsOnly), "a#b");
        assert_eq!(sanitize_file_name("???", SanitizePolicy::Strip), "Untitled");
    }

    #[test]
    fn percent() {
        assert_eq!(percent_decode("My%20Note%28x%29.md"), "My Note(x).md");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode(&percent_encode("a (b) 100%.md")), "a (b) 100%.md");
    }
}
