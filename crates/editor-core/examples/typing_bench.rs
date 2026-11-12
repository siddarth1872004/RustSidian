//! `cargo run --release --example typing_bench`: model-side cost of one keystroke
//! (rope edit + incremental re-parse + decorations for a 60-line viewport).
use editor_core::deco::{decorate, Mode};
use editor_core::Editor;
use std::time::Instant;

fn main() {
    let unit = "## Section\n\nSome *text* with a [[link|alias]] and #tag, `code` and **bold** words to fill the line.\n\n- item one\n- [ ] task two\n\n";
    for mb in [1usize, 5] {
        let text = unit.repeat((mb << 20) / unit.len());
        let mut e = Editor::new(&text);
        let at = text.len() / 2 + text[text.len() / 2..].find("text").unwrap();
        e.buf.set_sels(vec![editor_core::Sel::cursor(at)]);
        let n = 2000u32;
        let mut worst = std::time::Duration::ZERO;
        let t = Instant::now();
        for i in 0..n {
            let k = Instant::now();
            e.type_text("x", i as u64 * 10);
            let head = e.buf.sels[0].head;
            let b = e.buf.doc().block_index_at(head).unwrap_or(0);
            let d = decorate(e.buf.doc(), b.saturating_sub(15)..b + 15, &e.buf.sels, Mode::LivePreview);
            std::hint::black_box((b, d));
            worst = worst.max(k.elapsed());
        }
        println!("{mb} MB note: {:?} mean per keystroke, {worst:?} worst (edit + re-parse)", t.elapsed() / n);
    }
}
