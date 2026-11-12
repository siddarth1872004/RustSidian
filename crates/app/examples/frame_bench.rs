//! `cargo run --release -p walnut --example frame_bench -- <vault> <note>`
//! Keystroke-to-pixels cost in the application: type a character, then build, style, lay out
//! and rasterize the whole 1280x800 window (what the window loop does per keystroke).
use std::time::{Duration, Instant};
use walnut_app::{render, App, Key};
use walnut_ui::{Fonts, Pixmap};

fn main() {
    let mut args = std::env::args().skip(1);
    let vault = args.next().expect("vault");
    let note = args.next().expect("note");
    let mut app = App::open(std::path::Path::new(&vault)).unwrap();
    app.open_note(&note);
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1280, 800).unwrap();
    // Put the cursor in the middle of the note.
    let len = app.tab().unwrap().editor.buf.len();
    app.tab_mut().unwrap().editor.buf.set_sels(vec![editor_core::Sel::cursor(len / 2)]);
    app.key(Key::parse("ArrowDown").unwrap());
    render(&mut app, &mut fonts, &mut pm, 1.0);
    let mut times: Vec<Duration> = Vec::new();
    for i in 0..300 {
        let t = Instant::now();
        app.text_input(if i % 7 == 0 { " " } else { "x" });
        render(&mut app, &mut fonts, &mut pm, 1.0);
        times.push(t.elapsed());
    }
    times.sort();
    let scroll_t = Instant::now();
    for _ in 0..200 {
        app.scroll_editor(3);
        render(&mut app, &mut fonts, &mut pm, 1.0);
    }
    let scroll = scroll_t.elapsed() / 200;
    println!(
        "note {:.1} MB, {} files: keystroke-to-pixels median {:?}, p95 {:?}, max {:?}; scroll frame mean {:?}",
        len as f64 / 1e6,
        app.store.index.len(),
        times[times.len() / 2],
        times[times.len() * 95 / 100],
        times[times.len() - 1],
        scroll
    );
}
