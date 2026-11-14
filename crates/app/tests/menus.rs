use rustsidian_app::shell::Key;
use rustsidian_app::{render, App, Frame};
use rustsidian_ui::{Fonts, Pixmap};
use std::fs;

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (p, t) in [("A.md", "# A\nsee [[B]]\n"), ("B.md", "b\n"), ("dir/C.md", "c [[B]]\n")] {
        let abs = d.path().join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, t).unwrap();
    }
    d
}

fn center(frame: &Frame, action: &str) -> (f32, f32) {
    let n = frame.dom.nodes.iter().find(|n| n.el.action.as_deref() == Some(action)).unwrap();
    (n.rect.x + n.rect.w / 2.0, n.rect.y + n.rect.h / 2.0)
}

fn labels(app: &App) -> Vec<String> {
    app.menu
        .as_ref()
        .unwrap()
        .items
        .iter()
        .filter(|i| !i.action.is_empty())
        .map(|i| i.label.clone())
        .collect()
}

/// Click the menu row with this label.
fn pick(app: &mut App, fonts: &mut Fonts, pm: &mut Pixmap, label: &str) {
    let i = app.menu.as_ref().unwrap().items.iter().position(|i| i.label == label).unwrap();
    let frame = render(app, fonts, pm, 1.0);
    let (x, y) = center(&frame, &format!("menu:{i}"));
    app.click(&frame, fonts, x, y, false);
}

#[test]
fn file_menu_renames_and_updates_links() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("B");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1100, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let (x, y) = center(&frame, "open:B.md");
    app.context_menu(&frame, &mut fonts, x, y);
    assert!(labels(&app).iter().any(|l| l == "Delete"));
    pick(&mut app, &mut fonts, &mut pm, "Rename…");
    assert!(app.menu.is_none());
    assert_eq!(app.prompt.as_ref().unwrap().query, "B");
    app.key(Key::parse("Backspace").unwrap());
    app.text_input("Bee");
    app.key(Key::parse("Enter").unwrap());
    assert!(d.path().join("Bee.md").exists() && !d.path().join("B.md").exists());
    assert_eq!(app.tab().unwrap().path, "Bee.md");
    assert!(fs::read_to_string(d.path().join("A.md")).unwrap().contains("[[Bee]]"));
    assert!(fs::read_to_string(d.path().join("dir/C.md")).unwrap().contains("[[Bee]]"));
}

#[test]
fn folder_menu_creates_folders_and_notes() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.run("new-folder-in:dir");
    app.text_input("Inner");
    app.key(Key::parse("Enter").unwrap());
    assert!(d.path().join("dir/Inner/Untitled.md").exists());
    assert_eq!(app.tab().unwrap().path, "dir/Inner/Untitled.md");
    app.run("new-note-in:dir/Inner");
    assert_eq!(app.tab().unwrap().path, "dir/Inner/Untitled 1.md");
    app.run("duplicate:A.md");
    assert_eq!(fs::read_to_string(d.path().join("A 1.md")).unwrap(), "# A\nsee [[B]]\n");
    app.run("delete:dir");
    assert!(!d.path().join("dir").exists());
    assert!(app.tabs.iter().all(|t| !t.path.starts_with("dir/")));
}

#[test]
fn editor_menu_cut_copy_paste_without_system_clipboard() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("B");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1100, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let editor = &frame.dom.nodes[frame.info.editor.unwrap()].rect;
    app.context_menu(&frame, &mut fonts, editor.x + 80.0, editor.y + 20.0);
    let shown = labels(&app);
    assert!(
        ["Cut", "Copy", "Paste", "Bold", "Select all"].iter().all(|l| shown.iter().any(|s| s == l)),
        "{shown:?}"
    );
    // Escape closes the menu without running anything.
    app.key(Key::parse("Escape").unwrap());
    assert!(app.menu.is_none());
    // With nothing selected, cut takes the whole line.
    app.run("editor:cut");
    assert_eq!(app.tab().unwrap().editor.buf.text(), "");
    app.run("editor:paste");
    app.run("editor:paste");
    assert_eq!(app.tab().unwrap().editor.buf.text(), "b\nb\n");
    app.run("editor:select-all");
    app.run("editor:insert-wikilink");
    assert_eq!(app.tab().unwrap().editor.buf.text(), "[[b\nb\n]]");
}

#[test]
fn menu_keys_and_tab_menu() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    for n in ["A", "B", "dir/C"] {
        app.open_note(n);
    }
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1100, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let (x, y) = center(&frame, "tab:0");
    app.context_menu(&frame, &mut fonts, x, y);
    assert_eq!(labels(&app)[..3], ["Close", "Close others", "Close tabs to the right"]);
    // The menu is drawn and stays inside the window.
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let menu = frame.dom.nodes.iter().find(|n| n.el.classes.iter().any(|c| c == "menu")).unwrap();
    assert!(menu.rect.x + menu.rect.w <= 1100.0 && menu.rect.y + menu.rect.h <= 700.0);
    for k in ["ArrowDown", "ArrowDown", "ArrowDown", "Enter"] {
        app.key(Key::parse(k).unwrap());
    }
    assert!(app.menu.is_none());
    assert_eq!(app.tabs.iter().map(|t| t.path.as_str()).collect::<Vec<_>>(), ["A.md"]);
    // A click outside closes the menu and does nothing else.
    app.context_menu(&frame, &mut fonts, x, y);
    app.click(&frame, &mut fonts, 1090.0, 690.0, false);
    assert!(app.menu.is_none());
    assert_eq!(app.tabs.len(), 1);
}
