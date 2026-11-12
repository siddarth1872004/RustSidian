use ropey::Rope;
use std::time::Instant;

fn doc(bytes: usize) -> String {
    let line = "The quick brown fox [[jumps]] over the **lazy** dog #tag and `code` here.\n";
    line.repeat(bytes / line.len())
}

fn main() {
    for mb in [1usize, 5] {
        let text = doc(mb << 20);
        let t = Instant::now();
        let mut rope = Rope::from_str(&text);
        let build = t.elapsed();
        let mid = rope.len_chars() / 2;
        let n = 10_000;
        let t = Instant::now();
        for i in 0..n {
            rope.insert_char(mid + i, 'x');
        }
        let ins = t.elapsed() / n as u32;
        let t = Instant::now();
        let mut acc = 0usize;
        for i in 0..n {
            let l = rope.char_to_line(mid + i);
            acc += rope.line(l).len_bytes();
        }
        let look = t.elapsed() / n as u32;
        let t = Instant::now();
        let snap = rope.clone();
        let clone = t.elapsed();
        println!(
            "{mb} MB: build {build:?}, insert/keystroke {ins:?}, char->line+line fetch {look:?}, snapshot clone {clone:?} ({acc} {})",
            snap.len_bytes()
        );
    }
}
