//! Decorations for the editor views.
//!
//! Source mode styles syntax in place. Live preview additionally hides markup (`**`, `[[`, `# `)
//! except in the element the cursor is in, so the source is revealed where it is being edited.

use crate::buffer::Sel;
use md_engine::{Document, NodeId, NodeKind, Tree, NO_NODE};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    /// Markup that live preview does not draw.
    Hide,
    Heading(u8),
    Strong,
    Emph,
    Strike,
    Highlight,
    Code,
    CodeBlock,
    Math,
    Link,
    /// Destination part of a Markdown link (dimmed in source mode).
    Url,
    WikiLink,
    Embed,
    Tag,
    Quote,
    Callout,
    ListMarker,
    Task {
        checked: bool,
    },
    FrontMatter,
    Comment,
    Html,
    Footnote,
    BlockId,
    Rule,
    Table,
    /// Syntax characters shown dimmed (source mode, or revealed in live preview).
    Markup,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Deco {
    pub start: usize,
    pub end: usize,
    pub style: Style,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Source,
    LivePreview,
}

struct Cx<'a> {
    out: Vec<Deco>,
    base: usize,
    live: bool,
    sels: &'a [Sel],
}

impl Cx<'_> {
    fn push(&mut self, s: u32, e: u32, style: Style) {
        if e > s {
            self.out.push(Deco { start: self.base + s as usize, end: self.base + e as usize, style });
        }
    }
    /// Markup: hidden in live preview unless `reveal`, dimmed otherwise.
    fn markup(&mut self, s: u32, e: u32, reveal: bool) {
        self.push(s, e, if self.live && !reveal { Style::Hide } else { Style::Markup });
    }
    fn touches(&self, s: u32, e: u32) -> bool {
        let (s, e) = (self.base + s as usize, self.base + e as usize);
        self.sels.iter().any(|x| x.start() <= e && x.end() >= s)
    }
}

/// Decorations for the top-level blocks `blocks` (an index range), in document order.
pub fn decorate(doc: &Document, blocks: std::ops::Range<usize>, sels: &[Sel], mode: Mode) -> Vec<Deco> {
    let mut cx = Cx { out: Vec::new(), base: 0, live: mode == Mode::LivePreview, sels };
    for b in doc.blocks.get(blocks).unwrap_or(&[]) {
        cx.base = b.start;
        node(&mut cx, &b.tree, 0);
    }
    cx.out.sort_by_key(|d| (d.start, std::cmp::Reverse(d.end)));
    cx.out
}

fn kids_span(tree: &Tree, id: NodeId) -> Option<(u32, u32)> {
    let mut it = tree.children(id);
    let first = it.next()?.1.span.start;
    let last = tree.children(id).last()?.1.span.end;
    Some((first, last))
}

fn node(cx: &mut Cx<'_>, tree: &Tree, id: NodeId) {
    let n = *tree.node(id);
    let (s, e) = (n.span.start, n.span.end);
    let reveal = cx.touches(s, e);
    let mut recurse = true;
    match n.kind {
        NodeKind::Heading { level, setext } if level > 0 => {
            cx.push(s, e, Style::Heading(level));
            if !setext {
                if let Some((ks, ke)) = kids_span(tree, id) {
                    cx.markup(s, ks, reveal);
                    cx.markup(ke, e, reveal);
                } else {
                    cx.markup(s, e, true);
                }
            }
        }
        NodeKind::FrontMatter { .. } => cx.push(s, e, Style::FrontMatter),
        NodeKind::CodeBlock { content, fence, .. } => {
            cx.push(s, e, Style::CodeBlock);
            if fence > 0 {
                cx.markup(s, content.start, true);
                cx.markup(content.end, e, true);
            }
            recurse = false;
        }
        NodeKind::MathBlock { .. } => {
            cx.push(s, e, Style::Math);
            recurse = false;
        }
        NodeKind::BlockQuote => cx.push(s, e, Style::Quote),
        NodeKind::Callout { .. } => cx.push(s, e, Style::Callout),
        NodeKind::ListItem { task, marker } => {
            cx.push(marker.start, marker.end, Style::ListMarker);
            if let Some(t) = task {
                // `[x]` follows the marker and one space.
                cx.push(marker.end + 1, marker.end + 4, Style::Task { checked: t != b' ' });
            }
        }
        NodeKind::Table { .. } => cx.push(s, e, Style::Table),
        NodeKind::ThematicBreak => cx.push(s, e, Style::Rule),
        NodeKind::HtmlBlock | NodeKind::Html => cx.push(s, e, Style::Html),
        NodeKind::CommentBlock | NodeKind::Comment => cx.push(s, e, Style::Comment),
        NodeKind::FootnoteDef { .. } | NodeKind::FootnoteRef { .. } | NodeKind::InlineFootnote => {
            cx.push(s, e, Style::Footnote)
        }
        NodeKind::Emph | NodeKind::Strong | NodeKind::Strike | NodeKind::Highlight => {
            let style = match n.kind {
                NodeKind::Emph => Style::Emph,
                NodeKind::Strong => Style::Strong,
                NodeKind::Strike => Style::Strike,
                _ => Style::Highlight,
            };
            if let Some((ks, ke)) = kids_span(tree, id) {
                cx.push(ks, ke, style);
                cx.markup(s, ks, reveal);
                cx.markup(ke, e, reveal);
            }
        }
        NodeKind::Code { content } => {
            cx.push(content.start, content.end, Style::Code);
            cx.markup(s, content.start, reveal);
            cx.markup(content.end, e, reveal);
        }
        NodeKind::InlineMath { content, .. } => {
            cx.push(content.start, content.end, Style::Math);
            cx.markup(s, content.start, reveal);
            cx.markup(content.end, e, reveal);
        }
        NodeKind::Link { .. } | NodeKind::Image { .. } | NodeKind::RefLink { .. } => {
            let image = matches!(n.kind, NodeKind::Image { .. });
            match kids_span(tree, id) {
                Some((ks, ke)) => {
                    cx.push(ks, ke, if image { Style::Embed } else { Style::Link });
                    cx.markup(s, ks, reveal);
                    if cx.live && !reveal {
                        cx.push(ke, e, Style::Hide);
                    } else {
                        cx.push(ke, e, Style::Url);
                    }
                }
                None => cx.push(s, e, if image { Style::Embed } else { Style::Link }),
            }
        }
        NodeKind::WikiLink { target, subpath, alias, embed } => {
            let style = if embed { Style::Embed } else { Style::WikiLink };
            let inner_end = e.saturating_sub(2);
            let shown = if alias.is_empty() {
                (target.start, if subpath.is_empty() { target.end } else { subpath.end })
            } else {
                (alias.start, alias.end)
            };
            if cx.live && !reveal {
                cx.push(s, shown.0, Style::Hide);
                cx.push(shown.0, shown.1, style);
                cx.push(shown.1, e, Style::Hide);
            } else {
                cx.push(s, target.start, Style::Markup);
                cx.push(target.start, inner_end, style);
                cx.push(inner_end, e, Style::Markup);
            }
        }
        NodeKind::Tag => cx.push(s, e, Style::Tag),
        NodeKind::Autolink => cx.push(s, e, Style::Link),
        NodeKind::BlockId { .. } => cx.push(s, e, Style::BlockId),
        NodeKind::Escaped => cx.markup(s, s + 1, reveal),
        _ => {}
    }
    if recurse && n.first_child != NO_NODE {
        for (c, _) in tree.children(id) {
            node(cx, tree, c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, cursor: Option<usize>, mode: Mode) -> Vec<(String, Style)> {
        let doc = md_engine::parse(text);
        let sels: Vec<Sel> = cursor.map(Sel::cursor).into_iter().collect();
        decorate(&doc, 0..doc.blocks.len(), &sels, mode)
            .into_iter()
            .map(|d| (text[d.start..d.end].to_string(), d.style))
            .collect()
    }
    fn has(d: &[(String, Style)], t: &str, s: Style) -> bool {
        d.iter().any(|(x, y)| x == t && *y == s)
    }

    #[test]
    fn live_preview_hides_markup_away_from_cursor() {
        let text = "# Title\n\nsome **bold** and [[Note|alias]] `code` [t](http://u) #tag\n";
        let d = run(text, None, Mode::LivePreview);
        assert!(has(&d, "# ", Style::Hide) && has(&d, "# Title", Style::Heading(1)));
        assert!(has(&d, "bold", Style::Strong));
        assert_eq!(d.iter().filter(|(t, s)| t == "**" && *s == Style::Hide).count(), 2);
        assert!(
            has(&d, "[[Note|", Style::Hide)
                && has(&d, "alias", Style::WikiLink)
                && has(&d, "]]", Style::Hide)
        );
        assert!(
            has(&d, "code", Style::Code) && has(&d, "t", Style::Link) && has(&d, "](http://u)", Style::Hide)
        );
        assert!(has(&d, "#tag", Style::Tag));
    }

    #[test]
    fn cursor_reveals_only_its_element() {
        let text = "a **bold** b *it* c";
        let d = run(text, Some(5), Mode::LivePreview);
        assert_eq!(d.iter().filter(|(t, s)| t == "**" && *s == Style::Markup).count(), 2, "revealed");
        assert_eq!(
            d.iter().filter(|(t, s)| t == "*" && *s == Style::Hide).count(),
            2,
            "other element stays hidden"
        );
    }

    #[test]
    fn source_mode_never_hides() {
        let text = "# H\n\n- [x] done **b** [[L]]\n\n```rs\ncode\n```\n> q\n";
        let d = run(text, None, Mode::Source);
        assert!(d.iter().all(|(_, s)| *s != Style::Hide));
        assert!(has(&d, "-", Style::ListMarker) && has(&d, "[x]", Style::Task { checked: true }));
        assert!(has(&d, "```rs\ncode\n```", Style::CodeBlock) && has(&d, "> q", Style::Quote));
        assert!(has(&d, "L", Style::WikiLink));
    }

    #[test]
    fn only_requested_blocks() {
        let text = "**a**\n\n**b**\n\n**c**\n";
        let doc = md_engine::parse(text);
        let d = decorate(&doc, 1..2, &[], Mode::Source);
        assert!(d.iter().all(|x| x.start >= 7 && x.end <= 12));
        assert!(!d.is_empty());
    }
}
