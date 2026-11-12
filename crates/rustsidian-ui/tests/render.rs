use rustsidian_ui::{Dom, El, Env, Fonts, Theme};

const CSS: &str = "
body { --bg: #1e1e1e; --bg2: #262626; --text: #dcddde; --accent: hsl(258, 88%, 66%); background: var(--bg); color: var(--text); font-size: 14px; display: flex; flex-direction: column }
.main { display: flex; flex-grow: 1 }
.sidebar { width: 180px; background: var(--bg2); padding: 8px; border-right: 1px solid #3a3a3a; overflow: hidden }
.item { padding: 3px 6px; border-radius: 4px; white-space: nowrap }
.item.is-active { background: var(--accent); color: white }
.content { flex-grow: 1; padding: 16px 24px; overflow-y: auto }
h1 { font-size: 1.6em; font-weight: bold; margin-bottom: 8px }
.status { display: flex; justify-content: space-between; padding: 2px 8px; background: var(--bg2); font-size: 12px; color: #999 }
.modal { position: absolute; top: 40px; width: 240px; background: #333; border: 1px solid #555; border-radius: 8px; padding: 8px }
@media (prefers-color-scheme: light) { body { --bg: #ffffff; --bg2: #f2f3f5; --text: #222 } }
";

fn build() -> Dom {
    let mut d = Dom::new();
    let main = d.add(0, El::div().class("main"));
    let side = d.add(main, El::div().class("sidebar"));
    for (i, name) in
        ["Inbox", "Projects", "A very long note title that must be clipped", "Daily"].iter().enumerate()
    {
        d.add(
            side,
            El::div().class("item").class_if(i == 1, "is-active").text(*name).action(format!("open:{name}")),
        );
    }
    let content = d.add(main, El::div().class("content").id("content"));
    d.add(content, El::new("h1").text("Heading"));
    d.add(content, El::new("p").id("para").text("Some body text that is long enough to wrap onto more than one line when the pane is narrow, to exercise wrapping."));
    let status = d.add(0, El::div().class("status"));
    d.add(status, El::div().text("left"));
    d.add(status, El::div().id("right").text("12 words"));
    d.add(0, El::div().class("modal").id("modal").text("Palette"));
    d
}

#[test]
fn layout_geometry() {
    let mut theme = Theme::new();
    theme.add_sheet("test", CSS);
    let mut fonts = Fonts::new();
    let mut d = build();
    d.style(&theme, Env { dark: true, reduced_motion: false });
    d.layout(&mut fonts, 640.0, 400.0);
    let rect = |id: &str| d.nodes[d.by_id(id).unwrap()].rect;
    let side = d.nodes[2].rect;
    assert_eq!((side.x, side.y, side.w), (0.0, 0.0, 180.0));
    let status = d.nodes.iter().find(|n| n.el.classes.iter().any(|c| c == "status")).unwrap().rect;
    assert_eq!(status.y + status.h, 400.0, "status bar sits at the bottom");
    assert_eq!(side.h, status.y, "main row fills the remaining height");
    let content = rect("content");
    assert_eq!((content.x, content.w), (180.0, 460.0));
    let para = rect("para");
    assert_eq!(para.w, 460.0 - 48.0);
    assert!(para.h > 30.0, "paragraph wrapped: {para:?}");
    let right = rect("right");
    assert!((right.x + right.w - 632.0).abs() < 1.0, "space-between pushes the last item right: {right:?}");
    let modal = rect("modal");
    assert_eq!((modal.y, modal.w), (40.0, 240.0));
    assert_eq!(modal.x, 200.0, "absolute without left/right is centred");
    // Hit testing and actions.
    assert_eq!(d.action_at(20.0, 40.0).map(|a| a.1), Some("open:Projects"));
    assert_eq!(d.hit(300.0, 60.0), d.by_id("modal"), "the overlay is on top");
    assert!(d.action_at(400.0, 300.0).is_none());
    assert_eq!(d.scroller_at(400.0, 300.0), d.by_id("content"));
}

#[test]
fn renders_and_follows_color_scheme() {
    let mut theme = Theme::new();
    theme.add_sheet("test", CSS);
    let mut fonts = Fonts::new();
    assert!(fonts.font_count() > 0, "no fonts available for the render test");
    let mut px = |dark: bool| {
        let mut d = build();
        let pm = rustsidian_ui::render_to_pixmap(
            &mut d,
            &theme,
            Env { dark, reduced_motion: false },
            &mut fonts,
            640,
            400,
            1.0,
        )
        .unwrap();
        if let Ok(dir) = std::env::var("RUSTSIDIAN_SCREENSHOT_DIR") {
            pm.save_png(format!("{dir}/ui-{}.png", if dark { "dark" } else { "light" })).unwrap();
        }
        let at = |x: u32, y: u32| {
            let p = pm.pixel(x, y).unwrap();
            (p.red(), p.green(), p.blue())
        };
        // background of content, sidebar, active item, and whether any glyph pixels were drawn
        let glyphs = (200..600)
            .flat_map(|x| (20..50).map(move |y| (x, y)))
            .filter(|&(x, y)| at(x, y) != at(630, 300))
            .count();
        (at(630, 300), at(170, 300), at(170, 40), glyphs)
    };
    let (bg, side, active, glyphs) = px(true);
    assert_eq!(bg, (30, 30, 30));
    assert_eq!(side, (38, 38, 38));
    assert_eq!(active, (138, 92, 245));
    assert!(glyphs > 200, "heading text was drawn ({glyphs} px)");
    let (bg, side, _, _) = px(false);
    assert_eq!((bg, side), ((255, 255, 255), (242, 243, 245)));
}

#[test]
fn text_queries() {
    let mut fonts = Fonts::new();
    let style = rustsidian_ui::TextStyle {
        size: 14.0,
        line_height: 20.0,
        color: rustsidian_ui::Color::rgb(0, 0, 0),
        bold: false,
        italic: false,
        mono: false,
        wrap: true,
    };
    let text = "hello world\nsecond line";
    let (w, h) = fonts.measure(text, &[], &style, Some(500.0));
    assert!(w > 40.0 && (h - 40.0).abs() < 1.0, "{w}x{h}");
    let caret0 = fonts.range_rects(text, &[], &style, Some(500.0), 0, 0)[0];
    let caret5 = fonts.range_rects(text, &[], &style, Some(500.0), 5, 5)[0];
    let caret_l2 = fonts.range_rects(text, &[], &style, Some(500.0), 12, 12)[0];
    assert!(caret0.x == 0.0 && caret5.x > 20.0 && caret5.y == 0.0);
    assert!(caret_l2.x == 0.0 && (caret_l2.y - 20.0).abs() < 1.0);
    let sel = fonts.range_rects(text, &[], &style, Some(500.0), 6, 18);
    assert_eq!(sel.len(), 2, "a selection across two lines gives two rectangles");
    assert_eq!(fonts.hit(text, &[], &style, Some(500.0), caret5.x + 1.0, 5.0), 5);
    assert_eq!(fonts.hit(text, &[], &style, Some(500.0), 0.0, 25.0), 12);
    assert_eq!(fonts.hit(text, &[], &style, Some(500.0), 9999.0, 999.0), text.len());
    let (wrapped_w, wrapped_h) = fonts.measure("aaa bbb ccc ddd eee fff", &[], &style, Some(60.0));
    assert!(wrapped_w <= 61.0 && wrapped_h >= 60.0);
}
