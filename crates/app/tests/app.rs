use std::fs;
use walnut_app::shell::{Focus, ViewMode};
use walnut_app::{render, App, Key};
use walnut_ui::{Fonts, Pixmap};

fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let w = |p: &str, t: &str| {
        let abs = d.path().join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, t).unwrap();
    };
    w("Home.md", "# Home\n\nSee [[Second]] and **bold** text.\n");
    w("dir/Second.md", "Second note links [[Home]].\n");
    w("dir/Third.md", "third\n");
    w(
        ".obsidian/hotkeys.json",
        r#"{"editor:toggle-highlight":[{"modifiers":["Mod","Shift"],"key":"H"}],"editor:toggle-bold":[]}"#,
    );
    w(".obsidian/appearance.json", r#"{"theme":"moonstone","enabledCssSnippets":["s"]}"#);
    w(".obsidian/snippets/s.css", ".theme-light { --background-primary: #fefefe }");
    d
}

fn key(s: &str) -> Key {
    Key::parse(s).unwrap()
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        app.text_input(&c.to_string());
    }
}

#[test]
fn edit_save_and_conflict() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Home");
    assert_eq!(app.tab().unwrap().path, "Home.md");
    app.key(key("Ctrl+End"));
    typed(&mut app, "new line");
    assert!(app.tab().unwrap().editor.buf.is_modified());
    app.key(key("Ctrl+S"));
    assert!(fs::read_to_string(d.path().join("Home.md")).unwrap().ends_with("new line"));
    assert!(!app.tab().unwrap().editor.buf.is_modified());
    // An external edit must not be overwritten.
    typed(&mut app, "!");
    fs::write(d.path().join("Home.md"), "changed elsewhere, different length").unwrap();
    assert!(!app.save());
    assert!(app.notice.as_deref().unwrap().contains("outside Rustwalnut"));
    assert_eq!(fs::read_to_string(d.path().join("Home.md")).unwrap(), "changed elsewhere, different length");
    assert!(
        app.tab().unwrap().editor.buf.text().ends_with("new line!"),
        "the user's text is still in the tab"
    );
}

#[test]
fn external_change_reloads_clean_tab() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("dir/Third.md");
    fs::write(d.path().join("dir/Third.md"), "rewritten by another program\n").unwrap();
    app.files_changed(&["dir/Third.md".to_string()]);
    assert_eq!(app.tab().unwrap().editor.buf.text(), "rewritten by another program\n");
    fs::write(d.path().join("New.md"), "x").unwrap();
    app.files_changed(&["New.md".to_string()]);
    assert!(app.store.index.id("New.md").is_some());
}

#[test]
fn palette_switcher_and_hotkeys() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    assert!(!app.dark, "appearance.json theme=moonstone is light");
    app.key(key("Ctrl+P"));
    assert_eq!(app.focus, Focus::Prompt);
    typed(&mut app, "toggle light");
    assert_eq!(app.prompt.as_ref().unwrap().items[0].action, "theme:toggle-light-dark");
    app.key(key("Enter"));
    assert!(app.dark && app.prompt.is_none());

    app.key(key("Ctrl+O"));
    typed(&mut app, "secnd");
    assert_eq!(app.prompt.as_ref().unwrap().items[0].action, "open:dir/Second.md");
    app.key(key("Enter"));
    assert_eq!(app.tab().unwrap().path, "dir/Second.md");

    app.key(key("Ctrl+O"));
    typed(&mut app, "Brand New");
    app.key(key("ArrowDown"));
    app.key(key("ArrowDown"));
    app.key(key("ArrowDown"));
    app.key(key("Enter"));
    assert_eq!(app.tab().unwrap().path, "Brand New.md");
    assert!(d.path().join("Brand New.md").exists());

    // hotkeys.json: highlight rebound, bold unbound.
    typed(&mut app, "word");
    app.key(key("Ctrl+Shift+H"));
    assert_eq!(app.tab().unwrap().editor.buf.text(), "==word==");
    app.key(key("Ctrl+B"));
    assert_eq!(app.tab().unwrap().editor.buf.text(), "==word==", "Ctrl+B was unbound by hotkeys.json");
    assert_eq!(app.hotkey_for("editor:toggle-highlight"), "Ctrl+Shift+H");

    // Navigation history and tabs.
    app.key(key("Ctrl+Alt+ArrowLeft"));
    assert_eq!(app.tab().unwrap().path, "dir/Second.md");
    app.key(key("Ctrl+Alt+ArrowRight"));
    assert_eq!(app.tab().unwrap().path, "Brand New.md");
    app.key(key("Ctrl+W"));
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(
        fs::read_to_string(d.path().join("Brand New.md")).unwrap(),
        "==word==",
        "closing a tab saves it"
    );
}

#[test]
fn follow_link_search_and_modes() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Home");
    let at = app.tab().unwrap().editor.buf.text().find("Second").unwrap();
    app.tab_mut().unwrap().editor.buf.set_sels(vec![editor_core::Sel::cursor(at)]);
    app.run("editor:follow-link");
    assert_eq!(app.tab().unwrap().path, "dir/Second.md");

    app.key(key("Ctrl+Shift+F"));
    assert_eq!(app.focus, Focus::Search);
    typed(&mut app, "links -third");
    app.key(key("Enter"));
    assert_eq!(app.search_results.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(), vec!["dir/Second.md"]);
    app.search_query = "(".into();
    app.run_search();
    assert!(app.search_error.is_some());

    app.key(key("Escape"));
    app.key(key("Ctrl+E"));
    assert_eq!(app.tab().unwrap().mode, ViewMode::Reading);
    typed(&mut app, "ignored in reading mode");
    assert!(!app.tab().unwrap().editor.buf.is_modified());
}

#[test]
fn rendering_and_mouse() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1000, 600).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    // The snippet's light background is applied.
    let p = pm.pixel(700, 300).unwrap();
    assert_eq!((p.red(), p.green(), p.blue()), (254, 254, 254));
    // Click the "Home" row in the file explorer.
    let row = frame.dom.nodes.iter().find(|n| n.el.action.as_deref() == Some("open:Home.md")).unwrap().rect;
    app.click(&frame, &mut fonts, row.x + 20.0, row.y + 5.0, false);
    assert_eq!(app.tab().unwrap().path, "Home.md");

    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    // Live preview hides the `**` of "**bold**" while the cursor is elsewhere.
    let para = frame.info.blocks.iter().find(|b| b.display.contains("bold")).unwrap();
    assert_eq!(para.display, "See Second and bold text.");
    // Clicking on the word "bold" puts the cursor inside it in the *source*.
    let node = &frame.dom.nodes[para.node];
    let cb = frame.dom.content_rect(para.node);
    let rects = fonts.range_rects(
        &para.display,
        &node.el.spans,
        &node.style.text,
        Some(cb.w),
        para.display.find("old").unwrap(),
        para.display.find("old").unwrap() + 1,
    );
    app.click(&frame, &mut fonts, cb.x + rects[0].x + 1.0, cb.y + rects[0].y + 5.0, false);
    let head = app.tab().unwrap().editor.buf.sels[0].head;
    let src = app.tab().unwrap().editor.buf.text();
    assert_eq!(&src[head..head + 3], "old", "click maps through hidden markup");
    // With the cursor inside, the markup is revealed.
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(frame.info.blocks.iter().any(|b| b.display.contains("**bold**")));
    // Collapsing a folder hides its files.
    let before = app.tree_rows().len();
    app.run("toggle-folder:dir");
    render(&mut app, &mut fonts, &mut pm, 1.0);
    assert_eq!(app.tree_rows().len(), before - 2);
}

#[test]
fn cli_commands() {
    use walnut_app::cli::{run, Outcome};
    let d = vault();
    let v = d.path().to_string_lossy().to_string();
    let call = |args: &[&str]| {
        let mut out = Vec::new();
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let code = match run(&args, &mut out) {
            Outcome::Exit(c) => c,
            Outcome::Open { .. } => -1,
        };
        (code, String::from_utf8(out).unwrap())
    };
    let (code, out) = call(&["index", &v]);
    assert_eq!(code, 0);
    assert!(out.contains("notes: 3") && out.contains("resolved links: 2"), "{out}");
    let (code, out) = call(&["search", &v, "path:dir links"]);
    assert_eq!((code, out.lines().next()), (0, Some("dir/Second.md")));
    assert_eq!(call(&["search", &v, "(unclosed"]).0, 2);
    let html = d.path().join("out.html");
    let (code, _) = call(&["export", &v, "Home", "-o", &html.to_string_lossy()]);
    assert_eq!(code, 0);
    let page = fs::read_to_string(&html).unwrap();
    assert!(
        page.contains("<h1 data-heading=\"Home\">") && page.contains("href=\"dir/Second.html\""),
        "{page}"
    );
    assert_eq!(
        call(&["export", &v, "Home", "--format", "pdf"]).0,
        2,
        "PDF export is reported as not implemented"
    );
    let (code, out) = call(&["doctor", &v]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(call(&["plugin", "new"]).0, 2);
    assert_eq!(call(&[&v]).0, -1, "a bare vault path opens a window");
    assert_eq!(call(&["nonsense"]).0, 2);
    let (_, out) = call(&["commands"]);
    assert!(out.contains("command-palette:open"));
}

#[test]
fn block_widgets() {
    let d = tempfile::tempdir().unwrap();
    // A 2x1 opaque red PNG.
    let mut img = Pixmap::new(2, 1).unwrap();
    img.fill(tiny_skia_color(255, 0, 0));
    fs::write(d.path().join("red.png"), img.encode_png().unwrap()).unwrap();
    fs::write(d.path().join("Other.md"), "# Other\n\n## Part\n\nEmbedded **part**.\n\n## Tail\n\nhidden\n")
        .unwrap();
    fs::write(
        d.path().join("Main.md"),
        "Top\n\n> quoted\n> lines\n\n> [!warning]- Folded title\n> folded body\n\n> [!tip]\n> tip body\n\n\
         ![[Other#Part]] ![[red.png|40x20]]\n\n| A | B |\n|---|:-:|\n| **x** | y z |\n\nEnd\n",
    )
    .unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Main.md");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(1000, 800).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let shown: Vec<&str> = frame.info.blocks.iter().map(|b| b.display.as_str()).collect();
    // Quote markers are hidden; a folded callout shows only its title; an untitled one its type.
    assert!(shown.contains(&"quoted\nlines"), "{shown:?}");
    assert!(shown.contains(&"Folded title"), "{shown:?}");
    assert!(shown.contains(&"tip\ntip body"), "{shown:?}");
    // The table is a grid of cells without the delimiter row or pipes.
    for cell in ["A", "B", "x", "y z"] {
        assert!(shown.contains(&cell), "{shown:?}");
    }
    assert!(!shown.iter().any(|t| t.contains('|') || t.contains("---")), "{shown:?}");
    let texts: Vec<&str> = frame.dom.nodes.iter().filter_map(|n| n.el.text.as_deref()).collect();
    // The embed shows the addressed section only, formatted.
    assert!(texts.contains(&"Embedded part."), "{texts:?}");
    assert!(!texts.contains(&"hidden"));
    // The image is drawn at the requested size, in its own colour.
    let image = frame.dom.nodes.iter().find(|n| n.el.image.is_some()).unwrap();
    assert_eq!((image.rect.w, image.rect.h), (40.0, 20.0));
    let p = pm.pixel((image.rect.x + 20.0) as u32, (image.rect.y + 10.0) as u32).unwrap();
    assert_eq!((p.red(), p.green(), p.blue()), (255, 0, 0));

    // Clicking a cell puts the cursor in that cell's source, which turns the table back into text.
    let cell = frame.info.blocks.iter().find(|b| b.display == "y z").unwrap();
    let cb = frame.dom.content_rect(cell.node);
    app.click(&frame, &mut fonts, cb.x + 2.0, cb.y + 5.0, false);
    let head = app.tab().unwrap().editor.buf.sels[0].head;
    let src = app.tab().unwrap().editor.buf.text();
    assert!(src[head..].starts_with("y z"), "cursor at {:?}", &src[head..]);
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(frame.info.blocks.iter().any(|b| b.display.contains("|---|:-:|")));

    // Source mode shows everything as written and draws no widgets.
    app.tabs[0].mode = ViewMode::Source;
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(frame.info.blocks.iter().any(|b| b.display == "> quoted\n> lines"));
    assert!(!frame.dom.nodes.iter().any(|n| n.el.image.is_some()));

    // Reading mode renders widgets even where the cursor is.
    app.tabs[0].mode = ViewMode::Reading;
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(frame.info.blocks.iter().any(|b| b.display == "y z"));
}

#[test]
fn code_blocks_are_highlighted() {
    let (_d, mut app) = note("intro\n\n```rust\nfn main() { let s = \"x\"; } // c\n```\n", "{}");
    cursor_at(&mut app, "intro");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let block = frame.info.blocks.iter().find(|b| b.display.contains("fn main")).unwrap();
    let spans = &frame.dom.nodes[block.node].el.spans;
    let mut colors: Vec<_> = spans.iter().filter_map(|s| s.color).map(|c| (c.r, c.g, c.b)).collect();
    colors.sort_unstable();
    colors.dedup();
    assert!(colors.len() >= 4, "{colors:?}");
}

#[test]
fn math_blocks_are_typeset_and_editable() {
    let (_d, mut app) = note("intro\n\n$$\n\\frac{a}{b}\n$$\n\nAfter $x^2$ here.\n", "{}");
    cursor_at(&mut app, "intro");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert_eq!(frame.info.rich.len(), 2);
    for it in &frame.info.rich {
        assert!(!frame.dom.nodes[it.node].draw.is_empty());
    }
    let block = &frame.info.rich[0];
    let r = frame.dom.content_rect(block.node);
    assert!(r.h > 30.0);
    app.click(&frame, &mut fonts, r.x + r.w / 2.0, r.y + r.h / 2.0, false);
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert!(frame.info.blocks.iter().any(|b| b.display.contains("\\frac{a}{b}")));
    assert_eq!(frame.info.rich.len(), 1);
}

fn tiny_skia_color(r: u8, g: u8, b: u8) -> walnut_ui::SkColor {
    walnut_ui::SkColor::from_rgba8(r, g, b, 255)
}

fn note(text: &str, app_json: &str) -> (tempfile::TempDir, App) {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join(".obsidian")).unwrap();
    fs::write(d.path().join(".obsidian/app.json"), app_json).unwrap();
    fs::write(d.path().join("N.md"), text).unwrap();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("N.md");
    (d, app)
}

fn shown(app: &mut App) -> Vec<String> {
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 700).unwrap();
    render(app, &mut fonts, &mut pm, 1.0).info.blocks.iter().map(|b| b.display.clone()).collect()
}

fn cursor_at(app: &mut App, needle: &str) {
    let at = app.tab().unwrap().editor.buf.text().find(needle).unwrap();
    app.tab_mut().unwrap().editor.buf.set_sels(vec![editor_core::Sel::cursor(at)]);
}

#[test]
fn folding() {
    let (_d, mut app) = note("intro\n\n# A\nalpha\n\n## B\nbeta\n\n# C\n- item\n  - child\n- solo\n", "{}");
    cursor_at(&mut app, "alpha");
    // Folding from inside a section folds the section and moves the cursor to its heading.
    app.run("editor:toggle-fold");
    let s = shown(&mut app);
    assert!(s.iter().any(|t| t.starts_with("# A") && t.ends_with('⋯')), "{s:?}");
    assert!(!s.iter().any(|t| t.contains("alpha") || t.contains("beta")), "{s:?}");
    assert!(s.iter().any(|t| t.contains("C")), "{s:?}");
    assert_eq!(app.tab().unwrap().editor.buf.sels[0].head, 10);
    // Moving down skips the folded text.
    app.key(key("ArrowDown"));
    let head = app.tab().unwrap().editor.buf.sels[0].head;
    let text = app.tab().unwrap().editor.buf.text();
    assert!(head > text.find("beta").unwrap() + 4 && head <= text.find("# C").unwrap(), "head {head}");
    // Typing before a fold keeps it attached to its heading.
    cursor_at(&mut app, "intro");
    typed(&mut app, "x");
    let s = shown(&mut app);
    assert!(!s.iter().any(|t| t.contains("alpha")), "{s:?}");
    // Undo of that edit keeps it too; clicking the indicator unfolds.
    app.run("editor:undo");
    assert!(!shown(&mut app).iter().any(|t| t.contains("alpha")));
    app.run("fold:7");
    assert!(shown(&mut app).iter().any(|t| t.contains("alpha")));
    // A list item folds its children inside the same block.
    cursor_at(&mut app, "item");
    app.run("editor:toggle-fold");
    let s = shown(&mut app);
    assert!(s.iter().any(|t| t.contains("item") && !t.contains("child") && t.contains("solo")), "{s:?}");
    app.run("editor:fold-all");
    let s = shown(&mut app);
    assert_eq!(s.iter().filter(|t| !t.is_empty()).count(), 3, "{s:?}");
    app.run("editor:unfold-all");
    assert!(shown(&mut app).iter().any(|t| t.contains("child")));
}

#[test]
fn table_editing() {
    let (_d, mut app) = note("intro\n\n|a|b|\n|-|-:|\n|1|2|\n\nend\n", "{}");
    let text = |app: &App| app.tab().unwrap().editor.buf.text();
    cursor_at(&mut app, "1|2");
    // Tab moves to the next cell and re-aligns the source; past the last cell it adds a row.
    app.key(key("Tab"));
    assert_eq!(text(&app), "intro\n\n| a   |   b |\n| --- | --: |\n| 1   |   2 |\n\nend\n");
    typed(&mut app, "0");
    app.key(key("Tab"));
    typed(&mut app, "x");
    app.key(key("Shift+Tab"));
    typed(&mut app, "!");
    assert_eq!(text(&app), "intro\n\n| a   |   b |\n| --- | --: |\n| 1   |  20! |\n| x   |     |\n\nend\n");
    // Enter goes down a row in the same column.
    app.key(key("Enter"));
    typed(&mut app, "y");
    // Typing does not re-align; the next table operation does.
    assert!(text(&app).contains("| x   | y    |"), "{}", text(&app));
    app.run("editor:table-col-after");
    typed(&mut app, "new");
    app.run("editor:table-row-up");
    app.run("editor:table-col-left");
    app.run("editor:table-col-align-center");
    assert_eq!(
        text(&app),
        "intro\n\n| a   |     |   b |\n| --- | :-: | --: |\n| x   | new |   y |\n| 1   |     | 20! |\n\nend\n"
    );
    app.run("editor:table-row-delete");
    app.run("editor:table-col-delete");
    app.run("editor:table-sort-descending");
    assert_eq!(text(&app), "intro\n\n| a   |   b |\n| --- | --: |\n| 1   | 20! |\n\nend\n");
    // One undo step per operation that changed the text (the sort did not).
    app.run("editor:undo");
    assert!(text(&app).contains("| 1   |     | 20! |"), "{}", text(&app));
    app.run("editor:redo");
    // Outside a table, Tab indents as before and table commands do nothing.
    cursor_at(&mut app, "end");
    let before = text(&app);
    app.run("editor:table-row-after");
    assert_eq!(text(&app), before);
    app.run("editor:insert-table");
    assert!(text(&app).ends_with("end\n\n|     |     |\n| --- | --- |\n|     |     |\n"), "{:?}", text(&app));
}

#[test]
fn vim_keys() {
    let (_d, mut app) = note("one two\nthree\n", r#"{"vimMode":true}"#);
    let text = |app: &App| app.tab().unwrap().editor.buf.text();
    assert_eq!(app.tab().unwrap().vim_label(), Some("NORMAL"));
    // Normal mode: typed characters are commands.
    typed(&mut app, "dw");
    assert_eq!(text(&app), "two\nthree\n");
    typed(&mut app, "ihi ");
    assert_eq!(app.tab().unwrap().vim_label(), Some("INSERT"));
    assert_eq!(text(&app), "hi two\nthree\n");
    app.key(key("Escape"));
    assert_eq!(app.tab().unwrap().vim_label(), Some("NORMAL"));
    app.key(key("Enter"));
    typed(&mut app, "dd");
    assert_eq!(text(&app), "hi two\n");
    typed(&mut app, "u");
    assert_eq!(text(&app), "hi two\nthree\n");
    // The command turns vim keys off for open and new tabs.
    app.run("editor:toggle-vim");
    assert_eq!(app.tab().unwrap().vim_label(), None);
    typed(&mut app, "dd");
    assert!(text(&app).contains("dd"));
}

#[test]
fn graph_view() {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.open_note("Home");
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(900, 600).unwrap();
    app.key(key("Ctrl+G"));
    assert!(app.show_graph && app.tab().is_none());
    app.graph_view.as_mut().unwrap().settle();
    assert!(!app.graph_animating(), "the layout thread stops once settled");
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);
    let pane = frame.dom.nodes[frame.info.graph.unwrap()].rect;
    let g = app.graph_view.as_ref().unwrap();
    assert_eq!((g.graph.nodes.len(), g.graph.edges.len()), (3, 2));
    // Something other than the background is drawn inside the pane.
    let bg = pm.pixel(pane.x as u32 + 2, pane.y as u32 + 2).unwrap();
    let second = g.graph.nodes.iter().position(|n| &*n.label == "Second").unwrap();
    let (x, y) = g.node_screen(&pane, second).unwrap();
    assert!(pane.contains(x, y));
    assert_ne!(pm.pixel(x as u32, y as u32).unwrap(), bg);

    // Typing does not reach the hidden note.
    typed(&mut app, "zzz");
    assert!(!app.tabs[0].editor.buf.is_modified());

    // Hover highlights, wheel zooms about the pointer, dragging the background pans.
    assert!(app.hover(&frame, x, y));
    assert!(!app.hover(&frame, x, y));
    let zoom = app.graph_view.as_ref().unwrap().zoom;
    app.wheel(&frame, x, y, -3.0);
    let g = app.graph_view.as_ref().unwrap();
    assert!(g.zoom > zoom && !g.auto_fit);
    let (x2, y2) = g.node_screen(&pane, second).unwrap();
    assert!((x2 - x).abs() < 0.5 && (y2 - y).abs() < 0.5, "the point under the pointer stays put");
    app.run("graph:reset-view");
    let frame = render(&mut app, &mut fonts, &mut pm, 1.0);

    // Filters are saved to graph.json and rebuild the graph.
    app.run("graph:toggle-orphans");
    let frame2 = render(&mut app, &mut fonts, &mut pm, 1.0);
    assert_eq!(app.graph_view.as_ref().unwrap().graph.nodes.len(), 2);
    assert!(fs::read_to_string(d.path().join(".obsidian/graph.json"))
        .unwrap()
        .contains("\"showOrphans\": false"));
    drop(frame2);
    app.run("graph:toggle-orphans");
    app.graph_view.as_mut().unwrap().settle();
    let frame = {
        drop(frame);
        render(&mut app, &mut fonts, &mut pm, 1.0)
    };

    // A click on a node opens the note; the graph tab stays available.
    let g = app.graph_view.as_ref().unwrap();
    let second = g.graph.nodes.iter().position(|n| &*n.label == "Second").unwrap();
    let (x, y) = g.node_screen(&pane, second).unwrap();
    app.click(&frame, &mut fonts, x, y, false);
    app.release();
    assert!(!app.show_graph);
    assert_eq!(app.tab().unwrap().path, "dir/Second.md");
    app.run("graph:focus");
    assert!(app.show_graph);
    app.run("workspace:close");
    assert!(app.graph_view.is_none() && app.tabs.len() == 2);

    // Local graph: the active note and its neighbours.
    app.run("graph:open-local");
    let g = app.graph_view.as_ref().unwrap();
    assert_eq!(g.local.as_deref(), Some("dir/Second.md"));
    assert_eq!(g.graph.nodes.len(), 2);
}
