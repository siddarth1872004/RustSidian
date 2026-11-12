//! Performance budget gate. Generates synthetic vaults, measures, and exits non-zero if a
//! budget is exceeded.
//!
//! `cargo run --release -p walnut --example perf_budget -- [--dir <path>] [--sizes 1000,10000,50000] [--report-only]`
//!
//! Budgets that need a real window (cold start to a presented frame, idle RAM and CPU) are
//! measured by `scripts/window-budget.sh` instead; here start-up is measured headlessly as
//! "open vault + load fonts + render the first frame".

use search_core::{Corpus, Fuzzy, Query, RankWeights, Sort};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use walnut_app::{render, App};
use walnut_ui::{Fonts, Pixmap};

fn generate(root: &Path, n: usize) {
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
    let unit = "## Section\n\nSome *text* with a [[Note 5|alias]] and #tag, `code` and **bold** words to fill the line with realistic prose.\n\n- item one\n- [ ] task two\n\n";
    for mb in [1usize, 5] {
        fs::write(root.join(format!("Big{mb}.md")), unit.repeat((mb << 20) / unit.len())).unwrap();
    }
}

struct Report {
    rows: Vec<(String, String, String, bool)>,
}

impl Report {
    fn check(&mut self, what: String, measured: Duration, budget_ms: f64) {
        let ms = measured.as_secs_f64() * 1000.0;
        self.rows.push((what, format!("{ms:.2} ms"), format!("{budget_ms} ms"), ms <= budget_ms));
    }
}

fn best<T>(runs: usize, mut f: impl FnMut() -> T) -> (Duration, T) {
    let mut best: Option<(Duration, T)> = None;
    for _ in 0..runs {
        let t = Instant::now();
        let v = f();
        let d = t.elapsed();
        if best.as_ref().is_none_or(|b| d < b.0) {
            best = Some((d, v));
        }
    }
    best.unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let dir =
        value("--dir").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("walnut-perf-vaults"));
    let sizes: Vec<usize> = value("--sizes")
        .unwrap_or_else(|| "1000,10000,50000".into())
        .split(',')
        .filter_map(|s| s.parse().ok())
        .collect();
    let report_only = args.iter().any(|a| a == "--report-only");
    let mut r = Report { rows: Vec::new() };

    for &n in &sizes {
        let root = dir.join(format!("vault-{n}"));
        if !root.join("Big1.md").exists() {
            eprintln!("generating {n}-note vault in {}", root.display());
            generate(&root, n);
        }
        // Make sure the persisted index exists: start-up budgets assume a previously opened vault.
        drop(App::open(&root).unwrap());

        // Start-up, headless: open the vault while fonts load, then render the first frame.
        let (startup, (mut app, mut fonts)) = best(3, || {
            let fonts = std::thread::spawn(Fonts::new);
            let mut app = App::open(&root).unwrap();
            app.defer_graph = true;
            app.open_note("Note 42");
            let mut fonts = fonts.join().unwrap();
            let mut pm = Pixmap::new(1280, 800).unwrap();
            render(&mut app, &mut fonts, &mut pm, 1.0);
            (app, fonts)
        });
        match n {
            0..=5000 => r.check(format!("start to first frame, headless, {n} notes"), startup, 150.0),
            5001..=50_000 if n == 50_000 => {
                r.check(format!("start to first frame, headless, {n} notes"), startup, 400.0)
            }
            _ => r.rows.push((
                format!("start to first frame, headless, {n} notes"),
                format!("{:.2} ms", startup.as_secs_f64() * 1000.0),
                "(no budget at this size)".into(),
                true,
            )),
        }
        app.defer_graph = false;

        if n == *sizes.iter().max().unwrap_or(&0) {
            // Search and quick switcher on the largest vault.
            let corpus = Corpus::build(&app.store.index, &root);
            for q in [
                "filler text",
                "tag:#tag7 realistic",
                "line:(paragraph 3 alias)",
                "task-todo:open -tag:#tag1",
                "/note \\d+7 linking/",
                "[aliases:N77]",
            ] {
                let query = Query::parse(q).unwrap();
                let (t, _) = best(5, || {
                    corpus
                        .search(&app.store.index, &root, &query, Sort::Relevance, RankWeights::default(), 50)
                        .len()
                });
                r.check(format!("search `{q}`, {n} notes"), t, 30.0);
            }
            let paths: Vec<&str> = app.store.index.iter().map(|(_, f)| &*f.path).collect();
            let mut fuzzy = Fuzzy::new();
            for q in ["n", "no42", "note 1234"] {
                let (t, _) = best(5, || fuzzy.top(q, &paths, 20).len());
                r.check(format!("quick switcher `{q}`, {} paths", paths.len()), t, 5.0);
            }
        }

        if n == 10_000 || sizes.len() == 1 {
            // Typing and scrolling in large notes, full-window software render at 1280x800.
            let mut pm = Pixmap::new(1280, 800).unwrap();
            for (note, mb) in [("Big1", 1), ("Big5", 5)] {
                app.open_note(note);
                let len = app.tab().unwrap().editor.buf.len();
                app.tab_mut().unwrap().editor.buf.set_sels(vec![editor_core::Sel::cursor(len / 2)]);
                app.scroll_to_cursor();
                render(&mut app, &mut fonts, &mut pm, 1.0);
                let mut times: Vec<Duration> = (0..200)
                    .map(|i| {
                        let t = Instant::now();
                        app.text_input(if i % 7 == 0 { " " } else { "x" });
                        render(&mut app, &mut fonts, &mut pm, 1.0);
                        t.elapsed()
                    })
                    .collect();
                times.sort();
                if mb == 1 {
                    r.check("keystroke to pixels, 1 MB note (median)".into(), times[times.len() / 2], 8.0);
                    r.check(
                        "keystroke to pixels, 1 MB note (p95)".into(),
                        times[times.len() * 95 / 100],
                        8.0,
                    );
                } else {
                    let t = Instant::now();
                    for _ in 0..100 {
                        app.scroll_editor(3);
                        render(&mut app, &mut fonts, &mut pm, 1.0);
                    }
                    r.check(
                        "scroll frame, 5 MB note (mean; 60 Hz = 16.7 ms)".into(),
                        t.elapsed() / 100,
                        16.7,
                    );
                }
            }
        }
    }

    // Graph layout tick at 50,000 nodes. The tick runs on a worker thread, so it bounds layout
    // convergence speed, not pan/zoom frame rate.
    {
        use graph_core::model::{Graph, Node, NodeKind};
        let n = 50_000usize;
        let mut seed = 88172645463325252u64;
        let mut rnd = |m: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % m as u64) as u32
        };
        let mut g = Graph {
            nodes: (0..n)
                .map(|i| Node { kind: NodeKind::Note(i as u32), label: "".into(), degree: 0, group: None })
                .collect(),
            edges: (0..n * 3).map(|_| (rnd(n), rnd(n))).collect(),
        };
        for &(a, b) in &g.edges {
            g.nodes[a as usize].degree += 1;
            g.nodes[b as usize].degree += 1;
        }
        let mut layout = graph_core::Layout::new(n);
        let t = Instant::now();
        for _ in 0..20 {
            layout.step(&g, &graph_core::Forces::default());
        }
        r.rows.push((
            "graph layout tick, 50,000 nodes (worker thread)".into(),
            format!("{:.2} ms", t.elapsed().as_secs_f64() * 1000.0 / 20.0),
            "(informational)".into(),
            true,
        ));
    }

    println!("| Measurement | Result | Budget | |\n|---|---|---|---|");
    let mut failed = 0;
    for (what, measured, budget, ok) in &r.rows {
        println!("| {what} | {measured} | {budget} | {} |", if *ok { "ok" } else { "OVER" });
        failed += usize::from(!ok);
    }
    if failed > 0 && !report_only {
        eprintln!("{failed} budget(s) exceeded");
        std::process::exit(1);
    }
}
