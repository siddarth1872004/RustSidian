//! Property: after a rename plan is applied, every link points at the same file as before.

use proptest::prelude::*;
use vault_core::meta::extract;
use vault_core::{apply_edits, LinkFormat, Vault};

const DIRS: &[&str] = &["", "a", "a/b", "c", "My Dir"];
const NAMES: &[&str] = &["Note", "Other", "Deep One", "x", "Index", "Daily (1)"];

fn file_strategy() -> impl Strategy<Value = (usize, usize)> {
    (0..DIRS.len(), 0..NAMES.len())
}

fn path_of(d: usize, n: usize) -> String {
    if DIRS[d].is_empty() {
        format!("{}.md", NAMES[n])
    } else {
        format!("{}/{}.md", DIRS[d], NAMES[n])
    }
}

/// (target file index, style)
fn link_text(paths: &[String], from: &str, to: usize, style: u8) -> String {
    let target = &paths[to % paths.len()];
    let no_ext = target.trim_end_matches(".md");
    let name = no_ext.rsplit('/').next().unwrap();
    let rel = vault_core::path::relative(vault_core::path::parent(from), target);
    match style % 8 {
        0 => format!("[[{name}]]"),
        1 => format!("[[{no_ext}]]"),
        2 => format!("[[{name}#Heading|alias]]"),
        3 => format!("![[{no_ext}#^blk]]"),
        4 => format!("[text]({})", vault_core::path::percent_encode(target)),
        5 => format!("[text]({}#h)", vault_core::path::percent_encode(&rel)),
        6 => format!("[[{target}]]"),
        _ => format!("[[./{}]]", rel.trim_end_matches(".md")),
    }
}

fn resolved(v: &Vault) -> Vec<(String, Vec<Option<u32>>)> {
    let mut out: Vec<(String, Vec<Option<u32>>)> = v
        .iter()
        .map(|(_, f)| {
            let links = f.meta().map(|m| m.links.iter().map(|l| v.resolve(&l.target, &f.path)).collect());
            (f.path.to_string(), links.unwrap_or_default())
        })
        .collect();
    out.sort();
    out
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 1500, ..ProptestConfig::default() })]

    #[test]
    fn rename_preserves_link_targets(
        files in prop::collection::btree_set(file_strategy(), 2..10),
        links in prop::collection::vec((0usize..10, 0usize..10, 0u8..8), 0..25),
        which in 0usize..10,
        dest in file_strategy(),
        folder in any::<bool>(),
        format in 0u8..3,
    ) {
        let paths: Vec<String> = files.iter().map(|&(d, n)| path_of(d, n)).collect();
        let mut texts: Vec<String> = vec![String::from("intro\n"); paths.len()];
        for (from, to, style) in links {
            let from = from % paths.len();
            let l = link_text(&paths, &paths[from], to, style);
            texts[from].push_str(&l);
            texts[from].push_str(" and\n");
        }
        let mut v = Vault::new();
        v.link_format = [LinkFormat::Shortest, LinkFormat::Relative, LinkFormat::Absolute][format as usize];
        for (p, t) in paths.iter().zip(&texts) {
            v.upsert(p, 0, 0, Some(extract(t)));
        }
        let before = resolved(&v);
        let old = paths[which % paths.len()].clone();
        let (old, new) = if folder && old.contains('/') {
            (vault_core::path::parent(&old).to_string(), "moved/here".to_string())
        } else {
            (old, path_of(dest.0, dest.1))
        };
        // Renaming onto an existing file is rejected at the IO layer; skip it here.
        if v.id(&new).is_some() || paths.iter().any(|p| p.starts_with(&format!("{new}/"))) {
            return Ok(());
        }
        let plan = v.plan_rename(&old, &new);
        prop_assert!(!plan.moves.is_empty());
        // Apply the edits and re-index, as the IO layer does.
        let moved: std::collections::HashMap<&str, &str> = plan.moves.iter().map(|(o, n)| (n.as_str(), o.as_str())).collect();
        for fe in &plan.edits {
            let old_path = moved.get(fe.path.as_str()).copied().unwrap_or(fe.path.as_str());
            let idx = paths.iter().position(|p| p == old_path).unwrap();
            let new_text = apply_edits(&texts[idx], &fe.edits);
            // Only link targets change: the text outside links is untouched.
            prop_assert_eq!(new_text.matches(" and\n").count(), texts[idx].matches(" and\n").count());
            v.upsert(&fe.path, 0, 0, Some(extract(&new_text)));
        }
        // File ids are stable across the move, so the resolved targets must be identical.
        let rename = |p: &str| plan.moves.iter().find(|(o, _)| o == p).map_or(p.to_string(), |(_, n)| n.clone());
        let mut expected: Vec<(String, Vec<Option<u32>>)> = before.into_iter().map(|(p, l)| (rename(&p), l)).collect();
        expected.sort();
        prop_assert_eq!(resolved(&v), expected);
    }
}
