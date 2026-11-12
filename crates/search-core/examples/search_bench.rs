//! `cargo run --release --example search_bench -- <vault dir>`
use search_core::{Corpus, Fuzzy, Query, RankWeights, Sort};
use std::time::Instant;
use vault_core::VaultStore;

fn main() {
    let root = std::path::PathBuf::from(std::env::args().nth(1).expect("vault dir"));
    let (store, _) = VaultStore::open(&root).unwrap();
    let t = Instant::now();
    let corpus = Corpus::build(&store.index, &root);
    println!(
        "{} notes; corpus build {:?}, {:.1} MB in memory",
        store.index.len(),
        t.elapsed(),
        corpus.bytes() as f64 / 1e6
    );
    for q in [
        "filler text",
        "note 4242",
        "tag:#tag7 realistic",
        "line:(paragraph 3 alias)",
        "task-todo:open -tag:#tag1",
        "/note \\d+7 linking/",
        "[aliases:N77]",
        "zzzznotfound",
    ] {
        let query = Query::parse(q).unwrap();
        let mut best = std::time::Duration::MAX;
        let mut n = 0;
        for _ in 0..5 {
            let t = Instant::now();
            n = corpus.search(&store.index, &root, &query, Sort::Relevance, RankWeights::default(), 50).len();
            best = best.min(t.elapsed());
        }
        println!("  {q:32} top {n:2} hits in {best:?}");
    }
    let paths: Vec<&str> = store.index.iter().map(|(_, f)| &*f.path).collect();
    let mut fuzzy = Fuzzy::new();
    for q in ["n", "no42", "note 1234", "a07t11n3"] {
        let mut best = std::time::Duration::MAX;
        for _ in 0..5 {
            let t = Instant::now();
            let hits = fuzzy.top(q, &paths, 20);
            std::hint::black_box(hits);
            best = best.min(t.elapsed());
        }
        println!("  quick switcher {q:12} over {} paths: {best:?}", paths.len());
    }
}
