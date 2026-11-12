//! Typeset blocks that are painted with custom display-list commands: LaTeX math and Mermaid.
//!
//! The element only reserves the height (computed with font-independent metrics when the tree
//! is built); the commands are produced after layout, when real font metrics are available.

use md_engine::{NodeKind, Tree};
use walnut_ui::{Cmd, Color, Dom, El, Fonts, NodeId, Rect, TextStyle};

#[derive(Clone, Debug)]
pub enum Kind {
    Math {
        src: String,
        display: bool,
    },
    /// Inline formulas of a paragraph, typeset in a row below it.
    MathRow(Vec<String>),
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

/// The widget a top-level block is drawn as, if it is one. `src` is the text of the block's
/// root node and `root_start` that node's start within its top-level block.
pub fn block_kind(tree: &Tree, src: &str, root_start: u32) -> Option<Kind> {
    let NodeKind::MathBlock { content } = tree.root().kind else { return None };
    let text = src.get(
        content.start.checked_sub(root_start)? as usize..content.end.checked_sub(root_start)? as usize,
    )?;
    Some(Kind::Math { src: text.trim().to_string(), display: true })
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
        let mut fm = Fm { fonts, base: &base };
        match &it.kind {
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
