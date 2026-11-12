//! `cargo run --release --example open_bench -- <notes> [dir]`
//! Generates a synthetic vault (if missing) and measures cold index build and warm open.
use std::fs;
use std::path::PathBuf;
use std::time::Instant;
use vault_core::VaultStore;

fn gen(root: &std::path::Path, n: usize) {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut rnd = move |m: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % m as u64) as usize
    };
    for i in 0..n {
        let dir = root.join(format!("area{:02}/topic{:02}", i % 20, (i / 20) % 25));
        fs::create_dir_all(&dir).unwrap();
        let mut t =
            format!("---\ntags: [area{}, kind/{}]\naliases: [N{i}]\n---\n# Note {i}\n\n", i % 20, i % 7);
        for p in 0..6 {
            t.push_str(&format!(
                "Paragraph {p} of note {i} linking to [[Note {}]] and [[Note {}|alias]] with #tag{} and some filler text to make the note a realistic size for indexing benchmarks.\n\n",
                rnd(n), rnd(n), rnd(50)
            ));
        }
        t.push_str("## Tasks\n\n- [ ] open task\n- [x] done task ^blk\n");
        fs::write(dir.join(format!("Note {i}.md")), t).unwrap();
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let n: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(5000);
    let root = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("rustsidian-bench-{n}")));
    if !root.exists() {
        let t = Instant::now();
        gen(&root, n);
        eprintln!("generated {n} notes in {:?}", t.elapsed());
    }
    let _ = fs::remove_dir_all(root.join(".rustsidian"));
    let (mut s, st) = VaultStore::open(&root).unwrap();
    let t = Instant::now();
    s.save_cache().unwrap();
    let save = t.elapsed();
    println!(
        "{n} notes, no cache : total {:?} (walk {:?}, parse {:?}), save cache {save:?}",
        st.total, st.walk, st.parse
    );
    let mut best = None;
    for _ in 0..5 {
        let (mut s, st) = VaultStore::open(&root).unwrap();
        assert_eq!(st.parsed, 0);
        let t = Instant::now();
        let g = s.index.graph();
        let graph = t.elapsed();
        if best.as_ref().is_none_or(|(b, _, _): &(vault_core::OpenStats, _, _)| st.total < b.total) {
            best = Some((st, graph, g.out.iter().map(Vec::len).sum::<usize>()));
        }
    }
    let (st, graph, edges) = best.unwrap();
    let cache_mb = fs::metadata(VaultStore::cache_path(&root)).unwrap().len() as f64 / 1e6;
    println!(
        "{n} notes, warm open: total {:?} (walk {:?}, load cache {:?}), link graph {graph:?} ({edges} edges), cache {cache_mb:.1} MB",
        st.total, st.walk, st.load_cache
    );
}
