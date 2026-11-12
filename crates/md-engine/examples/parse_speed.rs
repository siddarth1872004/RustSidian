//! `cargo run --release --example parse_speed`: full-parse throughput and per-keystroke re-parse time.
use md_engine::{parse, Edit};
use std::time::Instant;

fn main() {
    let unit = "## Section\n\nSome *text* with a [[link|alias]] and #tag, `code` and **bold** words to fill the line.\n\n- item one\n- [ ] task two\n\n> quote line\n\n```rust\nfn main() {}\n```\n\n";
    for mb in [1usize, 5] {
        let mut text = unit.repeat((mb << 20) / unit.len());
        let t = Instant::now();
        let mut doc = parse(&text);
        let full = t.elapsed();
        let n = 2000;
        let mut at = text.len() / 2;
        at += text[at..].find("text").unwrap();
        let t = Instant::now();
        for _ in 0..n {
            text.insert(at, 'x');
            doc.edit(text.as_str(), Edit { start: at, old_end: at, new_end: at + 1 });
            at += 1;
        }
        let per = t.elapsed() / n;
        println!(
            "{mb} MB: full parse {full:?} ({:.0} MB/s, {} blocks); re-parse per keystroke {per:?} (includes String::insert memmove)",
            mb as f64 / full.as_secs_f64(),
            doc.blocks.len()
        );
    }
}
