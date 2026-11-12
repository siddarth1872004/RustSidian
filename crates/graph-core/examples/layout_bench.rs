//! `cargo run --release --example layout_bench`: layout tick time on synthetic graphs.
use graph_core::model::{Graph, Node, NodeKind};
use graph_core::{Forces, Layout};
use std::time::Instant;

fn main() {
    for n in [1_000usize, 10_000, 50_000] {
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
        let mut l = Layout::new(n);
        let t = Instant::now();
        let mut ticks = 0u32;
        while ticks < 30 && l.step(&g, &Forces::default()) {
            ticks += 1;
        }
        println!(
            "{n:6} nodes, {} edges: {:?} per layout tick (worker thread)",
            g.edges.len(),
            t.elapsed() / ticks
        );
    }
}
