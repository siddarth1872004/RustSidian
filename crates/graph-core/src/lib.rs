//! Note graphs for Rustwalnut.

pub mod layout;
pub mod model;
pub mod settings;

pub use layout::{Forces, Layout};
pub use model::{build, Filters, Graph, Local, Node, NodeKind, Sets};
pub use settings::{ColorGroup, GraphSettings};

use search_core::{Corpus, NoteView, Query};
use vault_core::Vault;

/// Evaluate a search query for every file, for the graph's search filter and color groups.
/// Without a corpus only metadata operators (`tag:`, `path:`, `file:`, properties) can match.
pub fn query_set(
    vault: &Vault,
    corpus: Option<&Corpus>,
    root: &std::path::Path,
    query: &str,
) -> Option<Vec<bool>> {
    let q = Query::parse(query).ok()?;
    let mut set = vec![false; vault.id_bound()];
    match corpus {
        Some(c) => {
            for hit in c.search(
                vault,
                root,
                &q,
                search_core::Sort::PathAsc,
                search_core::RankWeights::default(),
                usize::MAX,
            ) {
                set[hit.file as usize] = true;
            }
        }
        None => {
            for (id, f) in vault.iter() {
                let meta = (q.meta_parts() != 0).then(|| f.meta_parts(q.meta_parts())).flatten();
                let note = NoteView { path: &f.path, text: "", lower: "", meta: meta.as_deref() };
                set[id as usize] = q.matches(&note, &mut Vec::new());
            }
        }
    }
    Some(set)
}
