use rustsidian_app::welcome::{Screen, Welcome};
use rustsidian_app::{recent, render, App};
use rustsidian_ui::{Fonts, Pixmap};
use std::fs;

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (p, t) in [("A.md", "# A\n[[B]]\n"), ("B.md", "b\n"), ("dir/C.md", "c\n"), ("dir/sub/D.md", "d\n")] {
        let abs = d.path().join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, t).unwrap();
    }
    d
}

#[test]
fn tabs_close_and_reopen() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    for n in ["A", "B", "dir/C"] {
        app.open_note(n);
    }
    assert_eq!(app.tabs.len(), 3);
    assert_eq!(app.tab().unwrap().path, "dir/C.md");
    // Closing a tab before the active one keeps the same note active.
    app.run("close-tab:0");
    assert_eq!(app.tabs.iter().map(|t| t.path.as_str()).collect::<Vec<_>>(), ["B.md", "dir/C.md"]);
    assert_eq!(app.tab().unwrap().path, "dir/C.md");
    app.run("workspace:undo-close-pane");
    assert_eq!(app.tab().unwrap().path, "A.md");
    assert_eq!(app.tabs.len(), 3);
    app.run("workspace:close-others");
    assert_eq!(app.tabs.iter().map(|t| t.path.as_str()).collect::<Vec<_>>(), ["A.md"]);
    // Unsaved text is written when its tab closes.
    app.text_input("x");
    app.run("workspace:close-all");
    assert!(app.tabs.is_empty());
    assert!(fs::read_to_string(d.path().join("A.md")).unwrap().contains('x'));
    app.run("close-tab:7");
}

#[test]
fn close_button_middle_click_and_hover() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("A");
    app.open_note("B");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1000, 600).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let center = |action: &str| {
        let n = frame.dom.nodes.iter().find(|n| n.el.action.as_deref() == Some(action)).unwrap();
        (n.rect.x + n.rect.w / 2.0, n.rect.y + n.rect.h / 2.0)
    };
    // The close button sits inside its tab and wins the click.
    let (x, y) = center("close-tab:0");
    assert!(app.hover(&frame, x, y));
    assert_eq!(app.hover_action.as_deref(), Some("close-tab:0"));
    app.click(&frame, &mut fonts, x, y, false);
    assert_eq!(app.tabs.iter().map(|t| t.path.as_str()).collect::<Vec<_>>(), ["B.md"]);
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    // The hovered element and its ancestors carry the hover flag for `:hover` rules.
    assert!(frame.dom.nodes.iter().any(|n| n.hover && n.el.action.as_deref() == Some("close-tab:0")));
    assert!(frame.dom.nodes.iter().any(|n| n.hover && n.el.action.as_deref() == Some("tab:0")));
    let n = frame.dom.nodes.iter().find(|n| n.el.action.as_deref() == Some("tab:0")).unwrap();
    app.middle_click(&frame, n.rect.x + 20.0, n.rect.y + n.rect.h / 2.0);
    assert!(app.tabs.is_empty());
    // With nothing open, the pane offers what to do next.
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(frame.dom.nodes.iter().any(|n| n.el.classes.iter().any(|c| c == "empty-state-action")));
    // Every icon element was turned into drawing commands.
    let icons: Vec<_> =
        frame.dom.nodes.iter().filter(|n| n.el.attrs.iter().any(|(k, _)| k == "data-icon")).collect();
    assert!(icons.len() > 10 && icons.iter().all(|n| !n.draw.is_empty() || n.rect.w == 0.0));
}

#[test]
fn collapse_all_and_vault_switch_request() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(800, 600).unwrap();
    render(&mut app, &mut fonts, &mut pm, 1.0);
    let rows = app.tree_rows().len();
    app.run("file-explorer:collapse-all");
    render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(app.tree_rows().len() < rows);
    assert!(app.collapsed.contains("dir") && app.collapsed.contains("dir/sub"));
    app.run("file-explorer:collapse-all");
    render(&mut app, &mut fonts, &mut pm, 1.0);
    assert_eq!(app.tree_rows().len(), rows);
    assert!(!app.switch_vault);
    app.run("app:open-vault");
    assert!(app.switch_vault);
}

#[test]
fn animations_are_off_headless_and_finish_when_on() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(800, 600).unwrap();
    render(&mut app, &mut fonts, &mut pm, 1.0);
    app.run("app:toggle-left-sidebar");
    render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(!app.anims.active());
    app.anims.enabled = true;
    app.run("app:toggle-left-sidebar");
    render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(app.anims.active());
    assert!(app.anims.toward("left", true) < 1.0);
    std::thread::sleep(std::time::Duration::from_millis(260));
    assert!(!app.anims.active());
    assert_eq!(app.anims.toward("left", true), 1.0);
    // A notice times out by itself once the window can animate.
    app.notice = Some("hello".into());
    render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(app.notice_due_in().is_some_and(|ms| ms > 0));
}

#[test]
fn welcome_flow_and_recent_vaults() {
    // One test for everything that reads the configuration folder, since it is set per process.
    let cfg = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("RUSTSIDIAN_CONFIG_DIR", cfg.path());
    std::env::set_var("HOME", home.path());
    std::env::set_var("USERPROFILE", home.path());
    fs::create_dir_all(home.path().join("Notes/.obsidian")).unwrap();
    fs::create_dir_all(home.path().join("Plain")).unwrap();
    fs::create_dir_all(home.path().join(".hidden")).unwrap();

    let mut w = Welcome::new();
    assert!(w.recents.is_empty());
    assert_eq!(w.screen, Screen::Home);
    assert_eq!(
        w.folders.iter().map(|f| (f.name.as_str(), f.vault)).collect::<Vec<_>>(),
        [("Notes", true), ("Plain", false)]
    );
    // Open: go into a folder and choose it.
    assert!(w.run("welcome:open").is_none());
    assert!(w.run("enter:0").is_none());
    let chosen = w.run("welcome:choose").unwrap();
    assert!(chosen.ends_with("Notes"));
    assert!(w.run("welcome:up").is_none());
    assert_eq!(w.dir.as_deref(), Some(home.path()));
    // Create: needs a usable name, and refuses a folder that exists.
    w.run("welcome:create");
    assert!(w.run("welcome:choose").is_none() && w.error.is_some());
    w.text_input("Plain");
    assert!(w.key("Enter").is_none() && w.error.is_some());
    for _ in 0..5 {
        w.key("Backspace");
    }
    w.text_input("My vault");
    let made = w.key("Enter").unwrap();
    assert!(made.is_dir() && made.ends_with("My vault"));
    // The sample vault is created once and opens on its welcome note.
    let sample = w.run("welcome:sample").unwrap();
    assert_eq!(w.open_note.as_deref(), Some("Welcome.md"));
    assert!(sample.join("Getting started/Keyboard shortcuts.md").is_file());
    let mut app = App::open(&sample).unwrap();
    app.open_note("Welcome.md");
    assert_eq!(app.store.index.notes().count(), 4);
    assert_eq!(w.run("welcome:sample").unwrap(), sample);

    // Recent vaults: newest first, no duplicates, missing folders dropped.
    recent::remember(&chosen);
    recent::remember(&made);
    recent::remember(&chosen);
    let list = recent::load();
    assert_eq!(list.len(), 2);
    assert!(list[0].ends_with("Notes") && list[1].ends_with("My vault"));
    fs::remove_dir_all(&made).unwrap();
    assert_eq!(recent::load().len(), 1);
    let mut w = Welcome::new();
    assert_eq!(w.run("recent:0").unwrap(), list[0]);
    w.run("forget:0");
    assert!(w.recents.is_empty() && recent::load().is_empty());

    // It renders, and a click on the first action is routed.
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1280, 800).unwrap();
    let dom = w.render(&mut fonts, &mut pm, 1.0);
    let n = dom.nodes.iter().find(|n| n.el.action.as_deref() == Some("welcome:create")).unwrap();
    assert!(w.click(&dom, n.rect.x + 30.0, n.rect.y + 10.0).is_none());
    assert_eq!(w.screen, Screen::Create);
    assert!(w.hover(&dom, n.rect.x + 30.0, n.rect.y + 10.0));

    // Starting with no arguments asks for the welcome screen instead of printing usage.
    let mut out = Vec::new();
    assert!(matches!(rustsidian_app::cli::run(&[], &mut out), rustsidian_app::cli::Outcome::Welcome));
    assert!(matches!(
        rustsidian_app::cli::run(&["--help".to_string()], &mut out),
        rustsidian_app::cli::Outcome::Exit(0)
    ));
}

#[test]
fn app_icon_is_opaque_inside_and_clear_at_the_corners() {
    for size in [16u32, 64, 256] {
        let px = rustsidian_app::icon::rgba(size);
        assert_eq!(px.len(), (size * size * 4) as usize);
        assert_eq!(px[3], 0, "corner is transparent");
        let mid = ((size / 2 * size + size / 2) * 4) as usize;
        assert_eq!(px[mid + 3], 255, "centre is opaque");
    }
}
