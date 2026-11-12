//! Builds the element tree for the current application state, and maps editor geometry
//! (carets, selections, clicks) between source offsets and what is displayed.

use crate::shell::{App, Focus, Panel, PromptKind, ViewMode};
use editor_core::deco::{decorate, Deco, Style as Deco_};
use editor_core::Sel;
use md_engine::NodeKind;
use theme_engine::{parse_color, Color, Element, Env};
use vault_core::path;
use walnut_ui::{Dom, El, Fonts, NodeId, Rect, Span};

/// Colors the editor needs for syntax styling, read from the theme's CSS variables.
#[derive(Clone, Debug)]
pub struct Palette {
    pub accent: Color,
    pub muted: Color,
    pub faint: Color,
    pub code_bg: Color,
    pub code: Color,
    pub highlight: Color,
    pub selection: Color,
    pub caret: Color,
    pub tag: Color,
    pub heading_scale: [f32; 6],
    /// Syntax colors indexed by `highlight_core::Class as usize`.
    pub syntax: [Color; 14],
}

struct Body(bool);
impl Element for Body {
    fn tag(&self) -> &str {
        "body"
    }
    fn id(&self) -> Option<&str> {
        None
    }
    fn has_class(&self, c: &str) -> bool {
        c == if self.0 { "theme-dark" } else { "theme-light" }
    }
    fn attr(&self, _: &str) -> Option<&str> {
        None
    }
    fn parent(&self) -> Option<Self> {
        None
    }
    fn prev_sibling(&self) -> Option<Self> {
        None
    }
    fn is_last_child(&self) -> bool {
        false
    }
}

pub fn palette(app: &App) -> Palette {
    let class = if app.dark { "theme-dark" } else { "theme-light" };
    let c = app.theme.compute(
        &Body(app.dark),
        &[class],
        Env { dark: app.dark, reduced_motion: false },
        None,
        None,
    );
    let col = |name: &str, default: Color| c.var(name).and_then(|v| parse_color(&v)).unwrap_or(default);
    let num = |name: &str, default: f32| {
        c.var(name).and_then(|v| v.trim().trim_end_matches("em").parse().ok()).unwrap_or(default)
    };
    let syn = |name: &str, dark: (u8, u8, u8), light: (u8, u8, u8)| {
        let d = if app.dark { dark } else { light };
        col(name, Color::rgb(d.0, d.1, d.2))
    };
    let kw = syn("--code-keyword", (224, 108, 155), (170, 40, 100));
    let string = syn("--code-string", (140, 200, 120), (30, 120, 50));
    let comment = syn("--code-comment", (120, 120, 120), (140, 140, 140));
    let value = syn("--code-value", (170, 130, 240), (110, 60, 190));
    let func = syn("--code-function", (225, 200, 100), (150, 110, 10));
    let important = syn("--code-important", (235, 150, 80), (190, 90, 20));
    let property = syn("--code-property", (90, 200, 215), (10, 120, 150));
    let tag = syn("--code-tag", (230, 100, 100), (190, 40, 40));
    let operator = syn("--code-operator", (200, 130, 190), (150, 70, 140));
    let red = syn("--color-red", (230, 100, 100), (190, 40, 40));
    let mut syntax = [kw; 14];
    {
        use highlight_core::Class as K;
        for (k, v) in [
            (K::Keyword, kw),
            (K::String, string),
            (K::Comment, comment),
            (K::Number, value),
            (K::Function, func),
            (K::Type, important),
            (K::Property, property),
            (K::Tag, tag),
            (K::Operator, operator),
            (K::Important, important),
            (K::Constant, value),
            (K::Variable, property),
            (K::Inserted, string),
            (K::Deleted, red),
        ] {
            syntax[k as usize] = v;
        }
    }
    Palette {
        syntax,
        accent: col("--text-accent", Color::rgb(138, 92, 245)),
        muted: col("--text-muted", Color::rgb(150, 150, 150)),
        faint: col("--text-faint", Color::rgb(110, 110, 110)),
        code_bg: col("--code-background", Color::rgb(42, 42, 42)),
        code: col("--code-normal", Color::rgb(214, 197, 166)),
        highlight: col("--text-highlight-bg", Color { r: 255, g: 208, b: 0, a: 100 }),
        selection: col("--text-selection", Color { r: 138, g: 92, b: 245, a: 90 }),
        caret: col("--caret-color", Color::rgb(220, 220, 220)),
        tag: col("--tag-color", Color::rgb(138, 92, 245)),
        heading_scale: [
            num("--h1-size", 1.8),
            num("--h2-size", 1.5),
            num("--h3-size", 1.3),
            num("--h4-size", 1.15),
            num("--h5-size", 1.05),
            num("--h6-size", 1.0),
        ],
    }
}

/// One rendered editor block: what is displayed and how it maps back to the source.
#[derive(Clone, Debug, Default)]
pub struct BlockView {
    pub block: usize,
    pub node: NodeId,
    /// Source offset of the block content.
    pub src_start: usize,
    pub src_end: usize,
    pub display: String,
    /// (source offset, display offset, length) of each displayed segment.
    pub map: Vec<(usize, usize, usize)>,
}

impl BlockView {
    pub fn to_display(&self, src: usize) -> usize {
        let mut last = 0;
        for &(s, d, len) in &self.map {
            if src < s {
                return d;
            }
            if src <= s + len {
                return d + (src - s);
            }
            last = d + len;
        }
        last
    }
    pub fn to_source(&self, disp: usize) -> usize {
        for &(s, d, len) in &self.map {
            if disp <= d + len {
                return s + disp.saturating_sub(d);
            }
        }
        self.src_end
    }
}

#[derive(Default)]
pub struct ViewInfo {
    pub blocks: Vec<BlockView>,
    pub editor: Option<NodeId>,
    /// The graph pane and the hidden elements its colors are read from.
    pub graph: Option<NodeId>,
    pub graph_colors: Vec<NodeId>,
}

/// Syntax-highlight spans for a fenced code block, appended to `spans`. `map` is the
/// (source offset, display offset, length) mapping of the block's displayed segments.
fn code_spans(
    tree: &md_engine::Tree,
    block_start: usize,
    src: &str,
    src_start: usize,
    map: &[(usize, usize, usize)],
    pal: &Palette,
    spans: &mut Vec<Span>,
) {
    const MAX: usize = 256 * 1024;
    let root = tree.root();
    let NodeKind::CodeBlock { info, content, .. } = root.kind else { return };
    let (cs, ce) = (block_start + content.start as usize, block_start + content.end as usize);
    let (Some(text), Some(info)) =
        (src.get(cs.saturating_sub(src_start)..ce.saturating_sub(src_start)), src.get(
            (block_start + info.start as usize).saturating_sub(src_start)..(block_start + info.end as usize).saturating_sub(src_start),
        ))
    else {
        return;
    };
    if text.is_empty() || text.len() > MAX || cs < src_start {
        return;
    }
    let toks = highlight_core::highlight_cached(&highlight_core::lang_of_info(info), text);
    let conv = |abs: usize| {
        map.iter().find(|&&(s, _, len)| abs >= s && abs <= s + len).map(|&(s, d, _)| d + abs - s)
    };
    spans.retain(|s| s.color.is_some() || s.bold || s.italic || s.background.is_some());
    for t in toks.iter() {
        if let (Some(a), Some(b)) = (conv(cs + t.start), conv(cs + t.end)) {
            if b > a {
                let c = pal.syntax[t.class as usize];
                spans.push(Span { start: a, end: b, color: Some(c), scale: 1.0, ..Span::default() });
            }
        }
    }
    spans.sort_by_key(|s| s.start);
}

/// Turn a block's source and decorations into display text and styled spans.
pub fn block_display(
    src: &str,
    src_start: usize,
    decos: &[Deco],
    pal: &Palette,
) -> (String, Vec<Span>, Vec<(usize, usize, usize)>) {
    let end = src_start + src.len();
    let mut cuts: Vec<usize> = vec![src_start, end];
    for d in decos {
        cuts.push(d.start.clamp(src_start, end));
        cuts.push(d.end.clamp(src_start, end));
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut display = String::with_capacity(src.len());
    let mut spans = Vec::new();
    let mut map = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (ra, rb) = (a - src_start, b - src_start);
        if !src.is_char_boundary(ra) || !src.is_char_boundary(rb) {
            continue;
        }
        let mut span =
            Span { start: display.len(), end: display.len() + (b - a), scale: 1.0, ..Span::default() };
        let mut hidden = false;
        let mut styled = false;
        for d in decos.iter().filter(|d| d.start <= a && d.end >= b) {
            styled = true;
            match d.style {
                Deco_::Hide => hidden = true,
                Deco_::Heading(l) => {
                    span.bold = true;
                    span.scale = pal.heading_scale[(l.clamp(1, 6) - 1) as usize];
                }
                Deco_::Strong => span.bold = true,
                Deco_::Emph => span.italic = true,
                Deco_::Strike => {
                    span.strike = true;
                    span.color = Some(pal.muted);
                }
                Deco_::Highlight => span.background = Some(pal.highlight),
                Deco_::Code => {
                    span.mono = true;
                    span.color = Some(pal.code);
                    span.background = Some(pal.code_bg);
                }
                Deco_::Math => {
                    span.mono = true;
                    span.color = Some(pal.code);
                }
                Deco_::Link | Deco_::WikiLink | Deco_::Embed => {
                    span.color = Some(pal.accent);
                    span.underline = true;
                }
                Deco_::Tag => span.color = Some(pal.tag),
                Deco_::Url
                | Deco_::Markup
                | Deco_::Comment
                | Deco_::BlockId
                | Deco_::Html
                | Deco_::Footnote => span.color = Some(pal.faint),
                Deco_::ListMarker => span.color = Some(pal.muted),
                Deco_::Task { checked } => {
                    span.color = Some(if checked { pal.accent } else { pal.muted });
                    span.mono = true;
                }
                _ => {}
            }
        }
        if hidden {
            continue;
        }
        map.push((a, display.len(), b - a));
        display.push_str(&src[ra..rb]);
        if styled {
            spans.push(span);
        }
    }
    (display, spans, map)
}

fn sidebar_files(app: &App, dom: &mut Dom, parent: NodeId, viewport_h: f32) {
    dom.add(
        parent,
        El::div()
            .class("nav-header")
            .text(app.store.root.file_name().and_then(|n| n.to_str()).unwrap_or("Vault").to_string()),
    );
    let list = dom.add(parent, El::div().class("nav-files-container").id("files"));
    // Virtualised: fixed-height rows, only those in view are built; spacers keep the scroll range.
    let rows = app.tree_rows();
    let h = crate::shell::TREE_ROW_HEIGHT;
    let scroll = app.scrolls.get("files").copied().unwrap_or(0.0);
    let first = ((scroll / h).floor() as usize).min(rows.len());
    let last = (first + (viewport_h / h).ceil() as usize + 2).min(rows.len());
    let active = app.tab().map(|t| t.path.as_str());
    if first > 0 {
        dom.add(list, El::div().style(&format!("height: {}px", first as f32 * h)));
    }
    for r in &rows[first..last] {
        let pad = 8 + r.depth * 14 + if r.folder { 0 } else { 12 };
        let el = if r.folder {
            El::div()
                .class("nav-folder-title")
                .text(format!("{} {}", if r.collapsed { "▸" } else { "▾" }, r.label))
                .action(format!("toggle-folder:{}", r.path))
        } else {
            El::div()
                .class("nav-file-title")
                .class_if(active.is_some_and(|a| a.eq_ignore_ascii_case(&r.path)), "is-active")
                .text(r.label.clone())
                .action(format!("open:{}", r.path))
        };
        dom.add(list, el.style(&format!("padding-left: {pad}px")));
    }
    if last < rows.len() {
        dom.add(list, El::div().style(&format!("height: {}px", (rows.len() - last) as f32 * h)));
    }
}

fn sidebar_search(app: &App, dom: &mut Dom, parent: NodeId) {
    dom.add(parent, El::div().class("nav-header").text("Search"));
    let caret = if app.focus == Focus::Search { "▏" } else { "" };
    let shown = if app.search_query.is_empty() && app.focus != Focus::Search {
        "Type a query, press Enter".to_string()
    } else {
        format!("{}{caret}", app.search_query)
    };
    dom.add(parent, El::div().class("search-input-container").text(shown).action("focus:search"));
    let list = dom.add(parent, El::div().class("nav-files-container").id("search-results"));
    if let Some(e) = &app.search_error {
        dom.add(list, El::div().class("search-result-file-match").text(format!("Query error: {e}")));
    }
    for r in &app.search_results {
        dom.add(
            list,
            El::div()
                .class("search-result-file-title")
                .text(path::strip_md(&r.path).to_string())
                .action(format!("open:{}", r.path)),
        );
        for s in &r.snippets {
            dom.add(
                list,
                El::div()
                    .class("search-result-file-match")
                    .text(s.clone())
                    .action(format!("open:{}", r.path)),
            );
        }
    }
}

fn right_sidebar(app: &App, dom: &mut Dom, parent: NodeId) {
    let Some(tab) = app.tab() else { return };
    dom.add(parent, El::div().class("nav-header").text("Outline"));
    let outline = dom.add(parent, El::div().class("pane-section"));
    for h in tab.meta.iter().flat_map(|(_, m)| &m.headings) {
        dom.add(
            outline,
            El::div()
                .class("tree-item-self")
                .style(&format!("padding-left: {}px", 8 + (h.level as usize - 1) * 12))
                .text(h.text.to_string()),
        );
    }
    dom.add(parent, El::div().class("nav-header").text("Backlinks"));
    let back = dom.add(parent, El::div().class("pane-section"));
    if let Some(id) = app.store.index.id(&tab.path) {
        // The link graph is cached by the index; cloning the Arc is cheap.
        if let Some(g) = app.cached_graph() {
            for &src in g.back.get(id as usize).map(Vec::as_slice).unwrap_or(&[]) {
                if let Some(f) = app.store.index.entry(src) {
                    dom.add(
                        back,
                        El::div()
                            .class("tree-item-self")
                            .text(path::strip_md(&f.path).to_string())
                            .action(format!("open:{}", f.path)),
                    );
                }
            }
        }
    }
}

fn editor(app: &App, dom: &mut Dom, parent: NodeId, pal: &Palette, info: &mut ViewInfo) {
    let Some(tab) = app.tab() else {
        dom.add(
            parent,
            El::div()
                .class("empty-state")
                .text("No file is open. Ctrl+O opens the quick switcher, Ctrl+P the command palette."),
        );
        return;
    };
    let mode_label = match tab.mode {
        ViewMode::Source => "source",
        ViewMode::LivePreview => "live preview",
        ViewMode::Reading => "reading",
    };
    dom.add(
        parent,
        El::div().class("view-header").text(format!("{}  ·  {mode_label}", path::strip_md(&tab.path))),
    );
    let class = if tab.mode == ViewMode::Reading {
        "view-content markdown-reading-view"
    } else {
        "view-content markdown-source-view cm-editor"
    };
    let view = dom.add(parent, El::div().class(class).id("editor"));
    info.editor = Some(view);
    let buf = &tab.editor.buf;
    let doc = buf.doc();
    let first = tab.top_block.min(doc.blocks.len().saturating_sub(1));
    // Virtualised: only blocks from the scroll position onward, capped.
    let last = (first + 90).min(doc.blocks.len());
    let sels: &[Sel] = if tab.mode == ViewMode::Reading { &[] } else { &buf.sels };
    let decos = decorate(doc, first..last, sels, tab.deco_mode());
    for (i, b) in doc.blocks[first..last].iter().enumerate() {
        let root = b.tree.root();
        let (s, e) = (b.start + root.span.start as usize, b.start + root.span.end as usize);
        let class = match root.kind {
            NodeKind::Blank => {
                // Keep an empty line editable when the cursor is on it.
                if !sels.iter().any(|x| x.head >= b.start && x.head <= b.end()) {
                    continue;
                }
                "cm-line"
            }
            NodeKind::CodeBlock { .. } | NodeKind::MathBlock { .. } => "cm-line cm-codeblock",
            NodeKind::BlockQuote => "cm-line cm-quote",
            NodeKind::Callout { .. } => "cm-line cm-callout",
            NodeKind::FrontMatter { .. } => "cm-line cm-frontmatter",
            NodeKind::ThematicBreak
                if tab.mode != ViewMode::Source && !sels.iter().any(|x| x.head >= s && x.head <= e) =>
            {
                "cm-line cm-hr"
            }
            NodeKind::CommentBlock if tab.mode == ViewMode::Reading => continue,
            _ => "cm-line",
        };
        // Folded text: blocks inside a fold are skipped; a fold inside a block hides part of it.
        let hidden = &tab.hidden.1;
        if hidden.iter().any(|&(hs, he)| hs < s && he >= e) {
            continue;
        }
        let folded_here = hidden.iter().any(|&(hs, _)| hs >= s && hs <= e);
        let rendered = match tab.mode {
            ViewMode::Source => false,
            ViewMode::Reading => true,
            ViewMode::LivePreview => !sels.iter().any(|x| x.start() <= e && x.end() >= s),
        };
        let src = buf.slice(s, e);
        let mut block_decos: Vec<Deco> = decos.iter().filter(|d| d.start < e && d.end > s).copied().collect();
        for &(hs, he) in hidden.iter().filter(|&&(hs, he)| hs < e && he > s) {
            block_decos.push(Deco { start: hs.max(s), end: he.min(e), style: Deco_::Hide });
        }
        if rendered && matches!(root.kind, NodeKind::Table { .. }) {
            table(dom, view, pal, &src, s, &b.tree, b.start, first + i, &block_decos, info);
            continue;
        }
        let mut callout: Option<(Color, usize, usize)> = None;
        if rendered && matches!(root.kind, NodeKind::BlockQuote | NodeKind::Callout { .. }) {
            hide_quote_markers(&src, s, &mut block_decos);
            if let NodeKind::Callout { kind, fold, title } = root.kind {
                let first_end = s + src.find('\n').unwrap_or(src.len());
                let (ks, ke) = (b.start + kind.start as usize, b.start + kind.end as usize);
                let (ts, te) = (b.start + title.start as usize, b.start + title.end as usize);
                // Show the title, or the callout type when there is no title.
                let (shown_s, shown_e) = if te > ts { (ts, te) } else { (ks, ke) };
                let hide = |a: usize, b: usize| Deco { start: a, end: b, style: Deco_::Hide };
                block_decos.push(hide(s, shown_s));
                block_decos.push(hide(shown_e, first_end));
                block_decos.push(Deco { start: shown_s, end: shown_e, style: Deco_::Strong });
                if fold == md_engine::Fold::Closed {
                    block_decos.push(hide(first_end, e));
                }
                callout = Some((callout_color(&buf.slice(ks, ke)), shown_s, shown_e));
            }
        }
        let (mut display, mut spans, map) = if class.ends_with("cm-hr") {
            (String::new(), Vec::new(), Vec::new())
        } else {
            block_display(&src, s, &block_decos, pal)
        };
        if folded_here {
            let at = display.len();
            display.push_str("  ⋯");
            spans.push(Span {
                start: at,
                end: display.len(),
                color: Some(pal.faint),
                scale: 1.0,
                ..Span::default()
            });
        }
        let mut el =
            El::div().class(class).attr("data-block", &(first + i).to_string()).text(display.clone());
        let mut bv = BlockView { block: first + i, node: 0, src_start: s, src_end: e, display, map };
        if let Some((color, ts, te)) = callout {
            let (d0, d1) = (bv.to_display(ts), bv.to_display(te));
            for sp in spans.iter_mut().filter(|sp| sp.start >= d0 && sp.end <= d1) {
                sp.color = Some(color);
            }
            el = el.style(&format!(
                "border-left-color: rgb({0},{1},{2}); background-color: rgba({0},{1},{2},0.1)",
                color.r, color.g, color.b
            ));
        } else if matches!(root.kind, NodeKind::Callout { .. }) {
            let kind = match root.kind {
                NodeKind::Callout { kind, .. } => {
                    buf.slice(b.start + kind.start as usize, b.start + kind.end as usize)
                }
                _ => "".into(),
            };
            let c = callout_color(&kind);
            el = el.style(&format!("border-left-color: rgb({},{},{})", c.r, c.g, c.b));
        }
        code_spans(&b.tree, b.start, &src, s, &bv.map, pal, &mut spans);
        bv.node = dom.add(view, el.spans(spans));
        if matches!(root.kind, NodeKind::Heading { level, .. } if level > 0) {
            let folded = tab.folds.is_folded(b.start);
            dom.add(
                bv.node,
                El::div()
                    .class("cm-fold-indicator")
                    .class_if(folded, "is-collapsed")
                    .text(if folded { "▸" } else { "▾" })
                    .action(format!("fold:{}", b.start)),
            );
        }
        info.blocks.push(bv);
        if tab.mode != ViewMode::Source {
            embeds(app, dom, view, &tab.path, &b.tree, &buf.slice(b.start, b.end()));
        }
    }
}

/// Hide the `>` that starts each line of a quote or callout.
fn hide_quote_markers(src: &str, src_start: usize, decos: &mut Vec<Deco>) {
    let mut at = 0;
    for line in src.split_inclusive('\n') {
        let ws = line.len() - line.trim_start_matches(' ').len();
        if ws <= 3 && line[ws..].starts_with('>') {
            let n = ws + 1 + line[ws + 1..].starts_with(' ') as usize;
            decos.push(Deco { start: src_start + at, end: src_start + at + n, style: Deco_::Hide });
        }
        at += line.len();
    }
}

/// Obsidian's default colour for a callout type.
fn callout_color(kind: &str) -> Color {
    let (r, g, b) = match kind.trim().to_ascii_lowercase().as_str() {
        "abstract" | "summary" | "tldr" | "tip" | "hint" | "important" => (83, 223, 221),
        "success" | "check" | "done" => (68, 207, 110),
        "question" | "help" | "faq" | "warning" | "caution" | "attention" => (233, 151, 63),
        "failure" | "fail" | "missing" | "danger" | "error" | "bug" => (251, 70, 76),
        "example" => (168, 130, 255),
        "quote" | "cite" => (158, 158, 158),
        _ => (2, 122, 255),
    };
    Color { r, g, b, a: 255 }
}

/// A table drawn as a grid. Every cell is its own text block, so clicks map to the cell's source.
#[allow(clippy::too_many_arguments)]
fn table(
    dom: &mut Dom,
    view: NodeId,
    pal: &Palette,
    src: &str,
    src_start: usize,
    tree: &md_engine::Tree,
    block_start: usize,
    block: usize,
    decos: &[Deco],
    info: &mut ViewInfo,
) {
    struct Cell {
        start: usize,
        display: String,
        spans: Vec<Span>,
        map: Vec<(usize, usize, usize)>,
        end: usize,
        center: bool,
    }
    let mut rows: Vec<(bool, Vec<Cell>)> = Vec::new();
    for (rid, rn) in tree.children(0) {
        let NodeKind::TableRow { header } = rn.kind else { continue };
        let mut cells = Vec::new();
        for (_, cn) in tree.children(rid) {
            let NodeKind::TableCell { align } = cn.kind else { continue };
            let (a, b) = (block_start + cn.span.start as usize, block_start + cn.span.end as usize);
            let Some(raw) = src.get(a.saturating_sub(src_start)..b.saturating_sub(src_start)) else {
                continue;
            };
            let start = a + (raw.len() - raw.trim_start().len());
            let text = raw.trim();
            let cell_decos: Vec<Deco> =
                decos.iter().filter(|d| d.start < start + text.len() && d.end > start).copied().collect();
            let (display, spans, map) = block_display(text, start, &cell_decos, pal);
            cells.push(Cell {
                start,
                end: start + text.len(),
                display,
                spans,
                map,
                center: align == md_engine::Align::Center,
            });
        }
        rows.push((header, cells));
    }
    let cols = rows.iter().map(|(_, c)| c.len()).max().unwrap_or(0);
    if cols == 0 {
        return;
    }
    // Column widths follow the longest cell, so short columns stay narrow. The text is not
    // measured here; 10 px per character plus the cell padding is a deliberate over-estimate.
    let weights: Vec<f32> = (0..cols)
        .map(|c| {
            let chars =
                rows.iter().filter_map(|(_, r)| r.get(c)).map(|cell| cell.display.chars().count()).max();
            chars.unwrap_or(0).clamp(3, 60) as f32 * 10.0 + 22.0
        })
        .collect();
    let total: f32 = weights.iter().sum();
    let grid =
        dom.add(view, El::div().class("markdown-table").style(&format!("max-width: {:.0}px", total + 2.0)));
    for (header, cells) in rows {
        let row = dom.add(grid, El::div().class("table-row").class_if(header, "is-header"));
        for (c, cell) in cells.into_iter().enumerate() {
            let node = dom.add(
                row,
                El::div()
                    .class("table-cell")
                    .class_if(cell.center, "is-center")
                    .style(&format!("width: {:.3}%", weights[c] / total * 100.0))
                    .text(cell.display.clone())
                    .spans(cell.spans),
            );
            info.blocks.push(BlockView {
                block,
                node,
                src_start: cell.start,
                src_end: cell.end,
                display: cell.display,
                map: cell.map,
            });
        }
    }
}

const EMBED_MAX_BLOCKS: usize = 30;

/// The blocks of an embedded note, formatted like live preview without a cursor. Tables and
/// embeds inside an embed are shown as their source.
fn embedded_blocks(dom: &mut Dom, parent: NodeId, text: &str, action: &str, pal: &Palette) {
    let doc = md_engine::parse(text);
    let n = doc.blocks.len().min(EMBED_MAX_BLOCKS);
    let decos = decorate(&doc, 0..n, &[], editor_core::deco::Mode::LivePreview);
    for b in &doc.blocks[..n] {
        let root = b.tree.root();
        let (s, e) = (b.start + root.span.start as usize, b.start + root.span.end as usize);
        let class = match root.kind {
            NodeKind::Blank | NodeKind::CommentBlock | NodeKind::FrontMatter { .. } => continue,
            NodeKind::CodeBlock { .. } | NodeKind::MathBlock { .. } => "embed-line cm-codeblock",
            NodeKind::BlockQuote | NodeKind::Callout { .. } => "embed-line cm-quote",
            _ => "embed-line",
        };
        let Some(src) = text.get(s..e) else { continue };
        let mut block_decos: Vec<Deco> = decos.iter().filter(|d| d.start < e && d.end > s).copied().collect();
        if class.ends_with("cm-quote") {
            hide_quote_markers(src, s, &mut block_decos);
        }
        let (display, mut spans, map) = block_display(src, s, &block_decos, pal);
        code_spans(&b.tree, b.start, src, s, &map, pal, &mut spans);
        dom.add(parent, El::div().class(class).text(display).spans(spans).action(action));
    }
    if doc.blocks.len() > n {
        dom.add(parent, El::div().class("embed-line").text("…").action(action));
    }
}
const IMAGE_MAX_WIDTH: f32 = 640.0;

/// Images and note embeds referenced by a block, shown below it.
fn embeds(app: &App, dom: &mut Dom, view: NodeId, source: &str, tree: &md_engine::Tree, block_text: &str) {
    for (_, n) in tree.iter() {
        let (target, subpath, size) = match n.kind {
            NodeKind::WikiLink { target, subpath, alias, embed: true } => {
                (target.of(block_text).to_string(), subpath.of(block_text), alias.of(block_text))
            }
            NodeKind::Image { dest, .. } => {
                let d = dest.of(block_text);
                if d.contains("://") {
                    continue;
                }
                (path::percent_decode(d), "", "")
            }
            _ => continue,
        };
        let Some(id) = app.store.index.resolve(&target, source) else { continue };
        let Some(f) = app.store.index.entry(id) else { continue };
        let rel: &str = &f.path;
        if f.kind == vault_core::FileKind::Markdown {
            let Some(text) = app.media.borrow_mut().note(&app.store, rel) else { continue };
            use vault_core::cache::parts;
            let meta = f.meta_parts(parts::HEADINGS | parts::BLOCKS).unwrap_or_default();
            let section = crate::media::embed_section(&text, &meta, subpath);
            let action = format!("open:{rel}");
            let embed = dom.add(view, El::div().class("markdown-embed").action(action.clone()));
            dom.add(
                embed,
                El::div().class("markdown-embed-title").text(path::stem(rel)).action(action.clone()),
            );
            embedded_blocks(dom, embed, section, &action, &palette(app));
        } else if path::extension(rel).eq_ignore_ascii_case("png") {
            let Some(img) = app.media.borrow_mut().image(&app.store, rel) else { continue };
            let (iw, ih) = img.size();
            let (iw, ih) = (iw.max(1) as f32, ih.max(1) as f32);
            // `![[image.png|100]]` and `|100x80` set the size, as in Obsidian.
            let mut dims = size.split('x').map(|v| v.trim().parse::<f32>().ok().filter(|v| *v > 0.0));
            let (w, h) = match (dims.next().flatten(), dims.next().flatten()) {
                (Some(w), Some(h)) => (w, h),
                (Some(w), None) => (w, ih * w / iw),
                _ => (iw, ih),
            };
            let k = (IMAGE_MAX_WIDTH / w).min(1.0);
            dom.add(
                view,
                El::div()
                    .class("image-embed")
                    .style(&format!("width: {:.0}px; height: {:.0}px", w * k, h * k))
                    .image(img),
            );
        } else {
            dom.add(
                view,
                El::div()
                    .class("embed-unsupported")
                    .text(format!("{} (this file type cannot be shown yet)", path::file_name(rel))),
            );
        }
    }
}

/// Build the whole window.
fn graph(g: &crate::graphview::GraphView, dom: &mut Dom, parent: NodeId, info: &mut ViewInfo) {
    let header = dom.add(parent, El::div().class("view-header graph-controls"));
    dom.add(
        header,
        El::div().class("graph-count").text(format!(
            "{} nodes  ·  {} links",
            g.graph.nodes.len(),
            g.graph.edges.len()
        )),
    );
    let f = &g.settings.filters;
    let mut chips = vec![
        ("Tags", "graph:toggle-tags", f.show_tags),
        ("Attachments", "graph:toggle-attachments", f.show_attachments),
        ("Existing files only", "graph:toggle-unresolved", f.hide_unresolved),
        ("Orphans", "graph:toggle-orphans", f.show_orphans),
        ("Arrows", "graph:toggle-arrows", g.settings.show_arrows),
    ];
    let depth = format!("Depth {}", g.depth);
    if g.local.is_some() {
        chips.push((&depth, "graph:cycle-depth", true));
    }
    chips.push(("Fit", "graph:reset-view", g.auto_fit));
    for (label, cmd, on) in chips {
        dom.add(header, El::div().class("graph-control").class_if(on, "is-enabled").text(label).action(cmd));
    }
    let pane = dom.add(parent, El::div().class("view-content graph-view-container"));
    let colors = dom.add(pane, El::div().class("graph-view-colors"));
    info.graph_colors = crate::graphview::COLOR_CLASSES
        .iter()
        .map(|c| dom.add(colors, El::div().class("graph-view").class(c)))
        .collect();
    info.graph = Some(pane);
}

/// After layout: draw the graph into its pane.
pub fn add_graph(app: &mut App, dom: &mut Dom, info: &ViewInfo) {
    let (Some(pane), Some(g)) = (info.graph, &mut app.graph_view) else { return };
    let mut colors = [Color::rgb(128, 128, 128); 9];
    for (slot, &n) in colors.iter_mut().zip(&info.graph_colors) {
        let s = &dom.nodes[n].style;
        *slot = s.text.color.with_alpha(s.opacity);
    }
    let mut text = dom.nodes[pane].style.text.clone();
    text.size = 11.0;
    text.line_height = 14.0;
    dom.nodes[pane].draw = g.paint(dom.nodes[pane].rect, &colors, &text);
}

pub fn build(app: &App, viewport_h: f32) -> (Dom, ViewInfo) {
    let pal = palette(app);
    let mut info = ViewInfo::default();
    let mut dom = Dom::new();
    dom.nodes[0].el.classes.push(if app.dark { "theme-dark" } else { "theme-light" }.to_string());
    let ws = dom.add(0, El::div().class("workspace"));
    let ribbon = dom.add(ws, El::div().class("workspace-ribbon"));
    for (glyph, cmd, active) in [
        ("☰", "file-explorer:open", app.left_open && app.panel == Panel::Files),
        ("⌕", "global-search:open", app.left_open && app.panel == Panel::Search),
        ("⇄", "switcher:open", false),
        ("⌘", "command-palette:open", false),
        ("◐", "theme:toggle-light-dark", false),
    ] {
        dom.add(
            ribbon,
            El::div().class("side-dock-ribbon-action").class_if(active, "is-active").text(glyph).action(cmd),
        );
    }
    if app.left_open {
        let left = dom.add(ws, El::div().class("workspace-split mod-left-split"));
        match app.panel {
            Panel::Files => sidebar_files(app, &mut dom, left, viewport_h),
            Panel::Search => sidebar_search(app, &mut dom, left),
        }
    }
    let root = dom.add(ws, El::div().class("workspace-split mod-root"));
    let tabs = dom.add(root, El::div().class("workspace-tab-header-container"));
    for (i, t) in app.tabs.iter().enumerate() {
        let dirty = if t.editor.buf.is_modified() { " •" } else { "" };
        dom.add(
            tabs,
            El::div()
                .class("workspace-tab-header")
                .class_if(i == app.active && !app.show_graph, "is-active")
                .text(format!("{}{dirty}", t.title()))
                .action(format!("tab:{i}")),
        );
    }
    if let Some(g) = &app.graph_view {
        dom.add(
            tabs,
            El::div()
                .class("workspace-tab-header")
                .class_if(app.show_graph, "is-active")
                .text(g.title())
                .action("graph:focus"),
        );
    }
    match &app.graph_view {
        Some(g) if app.show_graph => graph(g, &mut dom, root, &mut info),
        _ => editor(app, &mut dom, root, &pal, &mut info),
    }
    if app.right_open {
        let right = dom.add(ws, El::div().class("workspace-split mod-right-split"));
        right_sidebar(app, &mut dom, right);
    }
    let status = dom.add(0, El::div().class("status-bar"));
    if let Some(t) = app.tab() {
        let words = t.meta.as_ref().map_or(0, |(_, m)| m.words);
        let backlinks = app
            .store
            .index
            .id(&t.path)
            .and_then(|id| app.cached_graph().map(|g| g.back.get(id as usize).map_or(0, Vec::len)))
            .unwrap_or(0);
        if let Some(label) = t.vim_label() {
            dom.add(status, El::div().class("status-bar-item").text(label));
        }
        dom.add(status, El::div().class("status-bar-item").text(format!("{backlinks} backlinks")));
        dom.add(status, El::div().class("status-bar-item").text(format!("{words} words")));
        dom.add(
            status,
            El::div()
                .class("status-bar-item")
                .text(format!("{} characters", t.editor.buf.rope().len_chars())),
        );
    }
    dom.add(
        status,
        El::div().class("status-bar-item").text(format!("{} notes", app.store.index.notes().count())),
    );
    if let Some(p) = &app.prompt {
        let modal = dom.add(0, El::div().class("prompt"));
        let placeholder = match p.kind {
            PromptKind::Commands => "Type a command…",
            PromptKind::Switcher => "Find or create a note…",
        };
        let shown = if p.query.is_empty() { placeholder.to_string() } else { format!("{}▏", p.query) };
        dom.add(modal, El::div().class("prompt-input").text(shown));
        let results = dom.add(modal, El::div().class("prompt-results"));
        for (i, item) in p.items.iter().enumerate() {
            let row = dom.add(
                results,
                El::div()
                    .class("suggestion-item")
                    .class_if(i == p.selected, "is-selected")
                    .action(item.action.clone()),
            );
            dom.add(row, El::div().class("suggestion-title").text(item.label.clone()));
            dom.add(row, El::div().class("suggestion-aux").text(item.aux.clone()));
        }
    }
    if let Some(n) = &app.notice {
        dom.add(0, El::div().class("notice").text(n.clone()));
    }
    if app.perf {
        dom.add(
            0,
            El::div().class("perf-overlay").text(format!(
                "frame {:.2} ms · {} nodes",
                app.last_frame_us as f32 / 1000.0,
                dom.nodes.len()
            )),
        );
    }
    for (id, y) in &app.scrolls {
        if let Some(n) = dom.by_id(id) {
            dom.nodes[n].scroll_y = *y;
        }
    }
    (dom, info)
}

/// After layout: add selection highlights and carets to the editor blocks.
pub fn add_carets(app: &App, dom: &mut Dom, info: &ViewInfo, fonts: &mut Fonts) {
    let Some(tab) = app.tab().filter(|t| t.mode != ViewMode::Reading) else { return };
    let pal = palette(app);
    let show_caret = app.focus == Focus::Editor;
    for bv in &info.blocks {
        let node = &dom.nodes[bv.node];
        let width = dom.content_rect(bv.node).w;
        let style = node.style.text.clone();
        let spans = node.el.spans.clone();
        let mut under = Vec::new();
        let mut over = Vec::new();
        // Inline backgrounds (highlight, inline code).
        for sp in spans.iter().filter(|s| s.background.is_some()) {
            for r in fonts.range_rects(&bv.display, &spans, &style, Some(width), sp.start, sp.end) {
                under.push((
                    Rect::new(r.x - 1.0, r.y + 1.0, r.w + 2.0, r.h - 2.0),
                    sp.background.unwrap_or(pal.code_bg),
                ));
            }
        }
        for sp in spans.iter().filter(|s| s.underline || s.strike) {
            for r in fonts.range_rects(&bv.display, &spans, &style, Some(width), sp.start, sp.end) {
                let y = if sp.strike { r.y + r.h * 0.55 } else { r.y + r.h - 3.0 };
                over.push((Rect::new(r.x, y, r.w, 1.0), sp.color.unwrap_or(style.color).with_alpha(0.7)));
            }
        }
        for sel in &tab.editor.buf.sels {
            if sel.end() < bv.src_start || sel.start() > bv.src_end {
                continue;
            }
            if !sel.is_empty() {
                let (a, b) =
                    (bv.to_display(sel.start().max(bv.src_start)), bv.to_display(sel.end().min(bv.src_end)));
                for r in fonts.range_rects(&bv.display, &spans, &style, Some(width), a, b) {
                    under.push((Rect::new(r.x, r.y, r.w, r.h), pal.selection));
                }
            }
            if show_caret && sel.head >= bv.src_start && sel.head <= bv.src_end {
                let d = bv.to_display(sel.head);
                if let Some(r) = fonts.range_rects(&bv.display, &spans, &style, Some(width), d, d).first() {
                    over.push((Rect::new(r.x, r.y + 2.0, 1.5, r.h - 4.0), pal.caret));
                }
            }
        }
        let n = &mut dom.nodes[bv.node];
        n.overlays = under;
        n.overlays_top = over;
    }
}

/// Source offset for a click at window coordinates, if it landed on an editor block.
pub fn offset_at(dom: &Dom, info: &ViewInfo, fonts: &mut Fonts, x: f32, y: f32) -> Option<usize> {
    let hit = dom.hit(x, y)?;
    let bv = info.blocks.iter().find(|b| b.node == hit).or_else(|| {
        // A click below the last block or in the margin goes to the nearest block by y.
        (Some(hit) == info.editor)
            .then(|| {
                info.blocks.iter().min_by_key(|b| {
                    let r = dom.nodes[b.node].rect;
                    ((r.y + r.h / 2.0 - y).abs() * 10.0) as i64
                })
            })
            .flatten()
    })?;
    let node = &dom.nodes[bv.node];
    let cb = dom.content_rect(bv.node);
    let d = fonts.hit(&bv.display, &node.el.spans, &node.style.text, Some(cb.w), x - cb.x, y - cb.y);
    Some(bv.to_source(d))
}
