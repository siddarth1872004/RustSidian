//! Typeset blocks that are painted with custom display-list commands: LaTeX math and Mermaid.
//!
//! The element only reserves the height (computed with font-independent metrics when the tree
//! is built); the commands are produced after layout, when real font metrics are available.

use md_engine::{NodeKind, Tree};
use rustsidian_ui::{Cmd, Color, Dom, El, Fonts, NodeId, Rect, TextStyle};

#[derive(Clone, Debug)]
pub enum Kind {
    Math {
        src: String,
        display: bool,
    },
    /// Inline formulas of a paragraph, typeset in a row below it.
    MathRow(Vec<String>),
    Mermaid(String),
}

#[derive(Clone, Debug)]
pub struct Item {
    pub node: NodeId,
    pub kind: Kind,
}

const BLOCK_SIZE: f32 = 20.0;
const ROW_SIZE: f32 = 17.0;
const PAD: f32 = 8.0;
const ROW_GAP: f32 = 28.0;
const BASELINE: f32 = 0.99;
const DIAGRAM_MAX_W: f32 = 760.0;

/// The widget a top-level block is drawn as, if it is one. `src` is the text of the block's
/// root node and `root_start` that node's start within its top-level block.
pub fn block_kind(tree: &Tree, src: &str, root_start: u32) -> Option<Kind> {
    match tree.root().kind {
        NodeKind::MathBlock { content } => {
            let text = src.get(
                content.start.checked_sub(root_start)? as usize
                    ..content.end.checked_sub(root_start)? as usize,
            )?;
            Some(Kind::Math { src: text.trim().to_string(), display: true })
        }
        NodeKind::CodeBlock { .. } => {
            let text = mermaid_source(tree, src, root_start)?;
            mermaid_core::render(text, &mut mermaid_core::Approx).ok()?;
            Some(Kind::Mermaid(text.to_string()))
        }
        _ => None,
    }
}

fn mermaid_source<'a>(tree: &Tree, src: &'a str, root_start: u32) -> Option<&'a str> {
    let NodeKind::CodeBlock { info, content, .. } = tree.root().kind else { return None };
    let rel = |v: u32| v.checked_sub(root_start).map(|v| v as usize);
    let lang = highlight_core::lang_of_info(src.get(rel(info.start)?..rel(info.end)?)?);
    if lang != "mermaid" {
        return None;
    }
    src.get(rel(content.start)?..rel(content.end)?)
}

/// Why a mermaid fence is shown as plain code, if it is one that cannot be drawn.
pub fn notice(tree: &Tree, src: &str, root_start: u32) -> Option<String> {
    let text = mermaid_source(tree, src, root_start)?;
    match mermaid_core::render(text, &mut mermaid_core::Approx) {
        Ok(_) => None,
        Err(mermaid_core::Error::Unsupported(k)) => {
            Some(format!("Mermaid `{k}` diagrams are not supported; showing the source"))
        }
        Err(mermaid_core::Error::Syntax(e)) => Some(format!("Mermaid: {e}; showing the source")),
    }
}

/// Source of the inline formulas of a paragraph-like block.
pub fn inline_math(tree: &Tree, block_text: &str) -> Vec<String> {
    tree.iter()
        .filter_map(|(_, n)| match n.kind {
            NodeKind::InlineMath { content, .. } => Some(content.of(block_text).trim().to_string()),
            _ => None,
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn approx_height(kind: &Kind) -> f32 {
    match kind {
        Kind::Math { src, display } => {
            math_core::render(src, BLOCK_SIZE, *display, &mut math_core::Approx).height() + 2.0 * PAD
        }
        Kind::Mermaid(src) => match mermaid_core::render(src, &mut mermaid_core::Approx) {
            Ok(d) => d.height * (DIAGRAM_MAX_W / d.width).min(1.0) + 2.0 * PAD,
            Err(_) => 0.0,
        },
        Kind::MathRow(items) => {
            items
                .iter()
                .map(|s| math_core::render(s, ROW_SIZE, false, &mut math_core::Approx).height())
                .fold(0.0, f32::max)
                + PAD
        }
    }
}

pub fn add(dom: &mut Dom, parent: NodeId, kind: Kind, info: &mut Vec<Item>) -> NodeId {
    let h = approx_height(&kind).ceil();
    let node = dom.add(parent, El::div().class("cm-line rich-block").style(&format!("height: {h}px")));
    info.push(Item { node, kind });
    node
}

struct Fm<'a> {
    fonts: &'a mut Fonts,
    base: &'a TextStyle,
}

impl math_core::Measure for Fm<'_> {
    fn width(&mut self, text: &str, size: f32, face: math_core::Face) -> f32 {
        let st = face_style(self.base, size, face);
        self.fonts.measure(text, &[], &st, None).0
    }
}

struct Dm<'a>(Fm<'a>);

impl mermaid_core::Measure for Dm<'_> {
    fn width(&mut self, text: &str, size: f32, bold: bool) -> f32 {
        let face = if bold { math_core::Face::Bold } else { math_core::Face::Roman };
        math_core::Measure::width(&mut self.0, text, size, face)
    }
}

fn mix(bg: Color, fg: Color, t: f32) -> Color {
    let l = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color { r: l(bg.r, fg.r), g: l(bg.g, fg.g), b: l(bg.b, fg.b), a: 255 }
}

fn background(dom: &Dom, mut n: NodeId) -> Color {
    loop {
        let c = dom.nodes[n].style.background;
        if c.a == 255 {
            return c;
        }
        match dom.nodes[n].parent {
            Some(p) => n = p,
            None => return Color { r: 30, g: 30, b: 30, a: 255 },
        }
    }
}

struct Pen {
    bg: Color,
    fg: Color,
    s: f32,
    ox: f32,
    oy: f32,
}

impl Pen {
    fn p(&self, x: f32, y: f32) -> (f32, f32) {
        (self.ox + x * self.s, self.oy + y * self.s)
    }
    fn fill(&self, f: mermaid_core::Fill) -> Color {
        match f {
            mermaid_core::Fill::Node => mix(self.bg, self.fg, 0.12),
            mermaid_core::Fill::Accent => mix(self.bg, self.fg, 0.05),
            _ => self.bg,
        }
    }
}

fn dashed(pts: &[(f32, f32)], on: f32, off: f32) -> Vec<[f32; 4]> {
    let mut out = Vec::new();
    let mut draw = true;
    let mut left = on;
    for w in pts.windows(2) {
        let (mut x, mut y) = w[0];
        let (dx, dy) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-3 {
            continue;
        }
        let (ux, uy) = (dx / len, dy / len);
        let mut rem = len;
        while rem > 0.0 {
            let step = rem.min(left);
            if draw {
                out.push([x, y, x + ux * step, y + uy * step]);
            }
            x += ux * step;
            y += uy * step;
            rem -= step;
            left -= step;
            if left <= 0.0 {
                draw = !draw;
                left = if draw { on } else { off };
            }
        }
    }
    out
}

fn emit_diagram(d: &mermaid_core::Diagram, pen: &Pen, base: &TextStyle, dm: &mut Dm, out: &mut Vec<Cmd>) {
    use mermaid_core::Item as I;
    let stroke = mix(pen.bg, pen.fg, 0.55);
    let line = mix(pen.bg, pen.fg, 0.8);
    let lw = |w: f32| (w * pen.s).max(1.0);
    for it in &d.items {
        match it {
            I::Rect { x, y, w, h, radius, fill, stroke: st } => {
                let snap = |v: f32| (v * 2.0).round() / 2.0;
                let (px, py) = pen.p(*x, *y);
                let (px, py) = (snap(px), snap(py));
                let (w, h, r) = (snap(w * pen.s), snap(h * pen.s), radius * pen.s);
                if *fill == mermaid_core::Fill::Background {
                    out.push(Cmd::Fill { rect: Rect::new(px, py, w, h), color: pen.bg, radius: r });
                    continue;
                }
                let inset = if *st { 1.0 } else { 0.0 };
                if *st {
                    out.push(Cmd::Fill { rect: Rect::new(px, py, w, h), color: stroke, radius: r });
                }
                out.push(Cmd::Fill {
                    rect: Rect::new(px + inset, py + inset, w - 2.0 * inset, h - 2.0 * inset),
                    color: pen.fill(*fill),
                    radius: (r - inset).max(0.0),
                });
            }
            I::Diamond { cx, cy, w, h, fill } => {
                let (cx, cy) = pen.p(*cx, *cy);
                let (hw, hh) = (w * pen.s / 2.0, h * pen.s / 2.0);
                let color = pen.fill(*fill);
                let mut y = -hh;
                while y < hh {
                    let half = hw * (1.0 - (y + 0.5).abs() / hh);
                    out.push(Cmd::Fill {
                        rect: Rect::new(cx - half, cy + y, 2.0 * half, 1.0),
                        color,
                        radius: 0.0,
                    });
                    y += 1.0;
                }
                let pts = [(cx - hw, cy), (cx, cy - hh), (cx + hw, cy), (cx, cy + hh), (cx - hw, cy)];
                let segs = pts.windows(2).map(|p| [p[0].0, p[0].1, p[1].0, p[1].1]).collect();
                out.push(Cmd::Lines { segs, width: lw(1.3), color: stroke });
            }
            I::Ellipse { cx, cy, rx, ry, fill } => {
                let (cx, cy) = pen.p(*cx, *cy);
                let r = rx.max(*ry) * pen.s;
                out.push(Cmd::Circles { items: vec![[cx, cy, r]], color: stroke });
                out.push(Cmd::Circles { items: vec![[cx, cy, r - 1.2]], color: pen.fill(*fill) });
            }
            I::Line { pts, dashed: dash, width } => {
                let pts: Vec<(f32, f32)> = pts.iter().map(|p| pen.p(p.0, p.1)).collect();
                let segs = if *dash {
                    dashed(&pts, 5.0 * pen.s, 4.0 * pen.s)
                } else {
                    pts.windows(2).map(|p| [p[0].0, p[0].1, p[1].0, p[1].1]).collect()
                };
                out.push(Cmd::Lines { segs, width: lw(*width), color: line });
            }
            I::Arrow { x, y, angle, .. } => {
                let (x, y) = pen.p(*x, *y);
                let (c, s) = (angle.cos(), angle.sin());
                let len = 9.0 * pen.s;
                let mut segs = Vec::new();
                let mut t = 0.0;
                while t <= len {
                    let half = t * 0.42;
                    let (bx, by) = (x - c * t, y - s * t);
                    segs.push([bx - s * half, by + c * half, bx + s * half, by - c * half]);
                    t += 0.75;
                }
                out.push(Cmd::Lines { segs, width: 1.2, color: line });
            }
            I::Text { x, y, text, bold, size } => {
                let (px, py) = pen.p(*x, *y);
                let sz = size * pen.s;
                let face = if *bold { math_core::Face::Bold } else { math_core::Face::Roman };
                let mut st = face_style(base, sz, face);
                st.color = pen.fg;
                let w = math_core::Measure::width(&mut dm.0, text, sz, face);
                out.push(Cmd::Text {
                    x: px - w / 2.0,
                    y: py - st.line_height / 2.0,
                    width: None,
                    text: text.clone(),
                    spans: Vec::new(),
                    style: st,
                });
            }
        }
    }
}

fn face_style(base: &TextStyle, size: f32, face: math_core::Face) -> TextStyle {
    let mut st = base.clone();
    st.size = size;
    st.line_height = size * 1.3;
    st.wrap = false;
    st.bold = face == math_core::Face::Bold;
    st.italic = face == math_core::Face::Italic;
    st.mono = face == math_core::Face::Mono;
    st
}

fn emit(l: &math_core::Layout, ox: f32, oy: f32, base: &TextStyle, color: Color, out: &mut Vec<Cmd>) {
    for g in &l.glyphs {
        let mut style = face_style(base, g.size, g.face);
        style.color = color;
        out.push(Cmd::Text {
            x: ox + g.x,
            y: oy + g.y - BASELINE * g.size,
            width: None,
            text: g.text.clone(),
            spans: Vec::new(),
            style,
        });
    }
    let mut widths: Vec<f32> = Vec::new();
    for s in &l.segs {
        if !widths.iter().any(|w| (*w - s.width).abs() < 0.01) {
            widths.push(s.width);
        }
    }
    for w in widths {
        let segs = l
            .segs
            .iter()
            .filter(|s| (s.width - w).abs() < 0.01)
            .map(|s| [ox + s.x0, oy + s.y0, ox + s.x1, oy + s.y1])
            .collect();
        out.push(Cmd::Lines { segs, width: w.max(1.0), color });
    }
}

/// Fill in the display lists of the rich blocks after layout.
pub fn paint(dom: &mut Dom, items: &[Item], fonts: &mut Fonts) {
    for it in items {
        let r: Rect = dom.content_rect(it.node);
        let st = &dom.nodes[it.node].style;
        let base = st.text.clone();
        let color = st.text.color.with_alpha(st.opacity);
        let mut cmds = Vec::new();
        let bg = background(dom, it.node);
        let mut fm = Fm { fonts, base: &base };
        if let Kind::Mermaid(src) = &it.kind {
            let mut dm = Dm(fm);
            if let Ok(d) = mermaid_core::render(src, &mut dm) {
                let sc = (DIAGRAM_MAX_W / d.width).min(1.0);
                let ox = r.x + ((r.w - d.width * sc) / 2.0).max(0.0);
                let pen = Pen { bg, fg: color, s: sc, ox, oy: r.y + PAD };
                emit_diagram(&d, &pen, &base, &mut dm, &mut cmds);
            }
            dom.nodes[it.node].draw = cmds;
            continue;
        }
        match &it.kind {
            Kind::Mermaid(_) => {}
            Kind::Math { src, display } => {
                let l = math_core::render(src, BLOCK_SIZE, *display, &mut fm);
                let x = r.x + ((r.w - l.width) / 2.0).max(0.0);
                let y = r.y + ((r.h - l.height()) / 2.0).max(0.0);
                emit(&l, x, y, &base, color, &mut cmds);
            }
            Kind::MathRow(rows) => {
                let mut x = r.x;
                for s in rows {
                    let l = math_core::render(s, ROW_SIZE, false, &mut fm);
                    emit(&l, x, r.y + PAD / 2.0, &base, color, &mut cmds);
                    x += l.width + ROW_GAP;
                }
            }
        }
        dom.nodes[it.node].draw = cmds;
    }
}
