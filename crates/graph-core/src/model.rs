//! Graph construction: nodes, edges, filters, local graphs, color groups.

use std::collections::HashMap;
use vault_core::{FileId, FileKind, Vault};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum NodeKind {
    Note(FileId),
    Attachment(FileId),
    Tag(Box<str>),
    /// A link target that does not exist yet.
    Unresolved(Box<str>),
}

#[derive(Clone, PartialEq, Debug)]
pub struct Node {
    pub kind: NodeKind,
    pub label: Box<str>,
    /// Number of edges touching the node; drives node size.
    pub degree: u32,
    /// Index of the first matching color group.
    pub group: Option<u16>,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    /// (source, target) node indices; direction is link direction.
    pub edges: Vec<(u32, u32)>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Filters {
    pub show_tags: bool,
    pub show_attachments: bool,
    /// Hide link targets that do not exist ("existing files only").
    pub hide_unresolved: bool,
    pub show_orphans: bool,
}

impl Default for Filters {
    fn default() -> Self {
        Filters { show_tags: false, show_attachments: false, hide_unresolved: false, show_orphans: true }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Local {
    pub center: FileId,
    pub depth: u32,
    pub backlinks: bool,
    pub forelinks: bool,
    /// Also show links between the neighbours themselves.
    pub interlinks: bool,
}

/// Per-file membership sets computed by the caller (search query filter, color group queries).
#[derive(Default)]
pub struct Sets {
    /// `filter[file]` is false for files excluded by the search filter.
    pub filter: Option<Vec<bool>>,
    /// One membership table per color group, in priority order.
    pub groups: Vec<Vec<bool>>,
}

fn label_of(path: &str, kind: FileKind) -> Box<str> {
    let name = vault_core::path::file_name(path);
    if kind == FileKind::Markdown { vault_core::path::stem(name) } else { name }.into()
}

pub fn build(vault: &mut Vault, filters: &Filters, sets: &Sets, local: Option<&Local>) -> Graph {
    let links = vault.graph();
    let allowed = |id: FileId| -> bool {
        let Some(f) = vault.entry(id) else { return false };
        if f.kind == FileKind::Attachment && !filters.show_attachments {
            return false;
        }
        sets.filter.as_ref().is_none_or(|s| s.get(id as usize).copied().unwrap_or(false))
    };

    // Which files are in scope.
    let scope: Vec<FileId> = match local {
        None => vault.iter().map(|(id, _)| id).filter(|&id| allowed(id)).collect(),
        Some(l) => {
            let mut dist: HashMap<FileId, u32> = HashMap::from([(l.center, 0)]);
            let mut frontier = vec![l.center];
            for d in 1..=l.depth {
                let mut next = Vec::new();
                for &f in &frontier {
                    let mut visit = |t: FileId| {
                        if allowed(t) && !dist.contains_key(&t) {
                            dist.insert(t, d);
                            next.push(t);
                        }
                    };
                    if l.forelinks {
                        links.out[f as usize].iter().for_each(|&(t, _)| visit(t));
                    }
                    if l.backlinks {
                        links.back[f as usize].iter().for_each(|&t| visit(t));
                    }
                }
                frontier = next;
            }
            let mut v: Vec<FileId> = dist.into_keys().collect();
            v.sort_unstable();
            v
        }
    };

    let mut g = Graph::default();
    let mut index: HashMap<NodeKind, u32> = HashMap::new();
    let mut node = |g: &mut Graph, kind: NodeKind, label: Box<str>, group: Option<u16>| -> u32 {
        *index.entry(kind.clone()).or_insert_with(|| {
            g.nodes.push(Node { kind, label, degree: 0, group });
            (g.nodes.len() - 1) as u32
        })
    };
    let file_node: HashMap<FileId, u32> = scope
        .iter()
        .filter_map(|&id| {
            let f = vault.entry(id)?;
            let kind =
                if f.kind == FileKind::Attachment { NodeKind::Attachment(id) } else { NodeKind::Note(id) };
            let group = sets
                .groups
                .iter()
                .position(|s| s.get(id as usize).copied().unwrap_or(false))
                .map(|g| g as u16);
            Some((id, node(&mut g, kind, label_of(&f.path, f.kind), group)))
        })
        .collect();

    let local_edge_ok = |l: &Local, from: FileId, to: FileId| -> bool {
        // Without interlinks, keep only edges on a path from the center.
        l.interlinks || from == l.center || to == l.center || l.depth > 1
    };
    for &id in &scope {
        let src = file_node[&id];
        for &(t, _) in &links.out[id as usize] {
            if t == id {
                continue;
            }
            if let Some(&dst) = file_node.get(&t) {
                if local.is_none_or(|l| local_edge_ok(l, id, t)) {
                    g.edges.push((src, dst));
                }
            }
        }
        if !filters.hide_unresolved {
            for name in &links.unresolved[id as usize] {
                let dst = node(&mut g, NodeKind::Unresolved(name.clone()), name.clone(), None);
                g.edges.push((src, dst));
            }
        }
        if filters.show_tags {
            if let Some(meta) = vault.entry(id).and_then(|f| f.meta()) {
                for tag in meta.all_tags() {
                    let key: Box<str> = tag.to_lowercase().into();
                    let dst = node(&mut g, NodeKind::Tag(key), format!("#{tag}").into(), None);
                    g.edges.push((src, dst));
                }
            }
        }
    }
    g.edges.sort_unstable();
    g.edges.dedup();
    for &(a, b) in &g.edges {
        g.nodes[a as usize].degree += 1;
        g.nodes[b as usize].degree += 1;
    }
    if !filters.show_orphans {
        let keep_center = local.and_then(|l| file_node.get(&l.center).copied());
        let mut remap = vec![u32::MAX; g.nodes.len()];
        let mut kept = Vec::new();
        for (i, n) in g.nodes.drain(..).enumerate() {
            if n.degree > 0 || keep_center == Some(i as u32) {
                remap[i] = kept.len() as u32;
                kept.push(n);
            }
        }
        g.nodes = kept;
        for e in &mut g.edges {
            *e = (remap[e.0 as usize], remap[e.1 as usize]);
        }
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    use vault_core::meta::extract;

    fn vault() -> Vault {
        let mut v = Vault::new();
        for (p, t) in [
            ("A.md", "[[B]] [[C]] [[Ghost]] ![[pic.png]] #topic"),
            ("B.md", "[[C]] #topic #other"),
            ("C.md", "[[A]]"),
            ("D.md", "[[E]]"),
            ("E.md", ""),
            ("Lonely.md", "no links"),
            ("pic.png", ""),
        ] {
            let meta = p.ends_with(".md").then(|| extract(t));
            v.upsert(p, 0, 0, meta);
        }
        v
    }

    fn labels(g: &Graph) -> Vec<&str> {
        let mut l: Vec<&str> = g.nodes.iter().map(|n| &*n.label).collect();
        l.sort_unstable();
        l
    }
    fn edge_labels(g: &Graph) -> Vec<(String, String)> {
        let mut e: Vec<_> = g
            .edges
            .iter()
            .map(|&(a, b)| (g.nodes[a as usize].label.to_string(), g.nodes[b as usize].label.to_string()))
            .collect();
        e.sort();
        e
    }

    #[test]
    fn global_graph_default_filters() {
        let mut v = vault();
        let g = build(&mut v, &Filters::default(), &Sets::default(), None);
        assert_eq!(labels(&g), vec!["A", "B", "C", "D", "E", "Ghost", "Lonely"]);
        let e = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(
            edge_labels(&g),
            vec![e("A", "B"), e("A", "C"), e("A", "Ghost"), e("B", "C"), e("C", "A"), e("D", "E")]
        );
        let a = g.nodes.iter().find(|n| &*n.label == "A").unwrap();
        assert_eq!(a.degree, 4);
    }

    #[test]
    fn filters_change_node_set() {
        let mut v = vault();
        let all =
            Filters { show_tags: true, show_attachments: true, hide_unresolved: true, show_orphans: false };
        let g = build(&mut v, &all, &Sets::default(), None);
        assert_eq!(labels(&g), vec!["#other", "#topic", "A", "B", "C", "D", "E", "pic.png"]);
        assert!(g.edges.iter().all(|&(a, b)| (a as usize) < g.nodes.len() && (b as usize) < g.nodes.len()));
        let topic = g.nodes.iter().find(|n| &*n.label == "#topic").unwrap();
        assert_eq!(topic.degree, 2);
    }

    #[test]
    fn search_filter_and_color_groups() {
        let mut v = vault();
        let keep = |names: &[&str]| -> Vec<bool> {
            let mut s = vec![false; v.id_bound()];
            for n in names {
                s[v.id(n).unwrap() as usize] = true;
            }
            s
        };
        let sets = Sets {
            filter: Some(keep(&["A.md", "B.md", "D.md"])),
            groups: vec![keep(&["B.md"]), keep(&["A.md", "B.md"])],
        };
        let g = build(&mut v, &Filters { hide_unresolved: true, ..Filters::default() }, &sets, None);
        assert_eq!(labels(&g), vec!["A", "B", "D"]);
        let group = |l: &str| g.nodes.iter().find(|n| &*n.label == l).unwrap().group;
        assert_eq!(
            (group("A"), group("B"), group("D")),
            (Some(1), Some(0), None),
            "first matching group wins"
        );
    }

    #[test]
    fn local_graph_depth_and_direction() {
        let mut v = vault();
        let center = v.id("B.md").unwrap();
        let f = Filters { hide_unresolved: true, ..Filters::default() };
        let l = |depth, backlinks, forelinks| Local { center, depth, backlinks, forelinks, interlinks: true };
        let g = build(&mut v, &f, &Sets::default(), Some(&l(1, true, true)));
        assert_eq!(labels(&g), vec!["A", "B", "C"]);
        let g = build(&mut v, &f, &Sets::default(), Some(&l(1, false, true)));
        assert_eq!(labels(&g), vec!["B", "C"]);
        let g = build(&mut v, &f, &Sets::default(), Some(&l(2, false, true)));
        assert_eq!(labels(&g), vec!["A", "B", "C"], "B -> C -> A");
        let g = build(&mut v, &f, &Sets::default(), Some(&Local { interlinks: false, ..l(1, true, true) }));
        assert!(
            !edge_labels(&g).contains(&("A".to_string(), "C".to_string())),
            "no neighbour-to-neighbour edges"
        );
        // The center stays even when it is an orphan and orphans are hidden.
        let lonely = v.id("Lonely.md").unwrap();
        let g = build(
            &mut v,
            &Filters { show_orphans: false, ..f },
            &Sets::default(),
            Some(&Local { center: lonely, ..l(2, true, true) }),
        );
        assert_eq!(labels(&g), vec!["Lonely"]);
    }
}
