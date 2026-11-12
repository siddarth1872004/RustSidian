//! HTML rendering of a parsed document (reading mode, export, snapshot tests).

use crate::{Align, Document, Fold, NodeId, NodeKind, Tree};
use std::collections::HashMap;
use std::fmt::Write;

pub fn escape(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    escape(s, &mut o);
    o
}

/// Hook for turning link targets into URLs. The default keeps targets as written.
pub trait LinkResolver {
    /// `target` is the wikilink target without subpath. Returns the href, or `None` if unresolved.
    fn resolve(&self, target: &str) -> Option<String>;
}

pub struct KeepLinks;
impl LinkResolver for KeepLinks {
    fn resolve(&self, target: &str) -> Option<String> {
        Some(target.to_string())
    }
}

pub fn render(doc: &Document, text: &str) -> String {
    render_with(doc, text, &KeepLinks)
}

pub fn render_with(doc: &Document, text: &str, resolver: &dyn LinkResolver) -> String {
    let mut r = R {
        out: String::with_capacity(text.len() * 3 / 2),
        refs: HashMap::new(),
        resolver,
        footnotes: Vec::new(),
    };
    // Link reference definitions and footnote definitions can appear anywhere.
    for b in &doc.blocks {
        let bt = b.text(text);
        for (id, n) in b.tree.iter() {
            match n.kind {
                NodeKind::LinkRefDef { label, dest, .. } => {
                    r.refs.entry(label.of(bt).to_lowercase()).or_insert_with(|| dest.of(bt).to_string());
                }
                NodeKind::FootnoteDef { label } => {
                    let mut body = String::new();
                    std::mem::swap(&mut body, &mut r.out);
                    for (c, _) in b.tree.children(id) {
                        r.block(&b.tree, c, bt, false);
                    }
                    std::mem::swap(&mut body, &mut r.out);
                    r.footnotes.push((label.of(bt).to_string(), body));
                }
                _ => {}
            }
        }
    }
    for b in &doc.blocks {
        r.block(&b.tree, 0, b.text(text), false);
    }
    if !r.footnotes.is_empty() {
        r.out.push_str("<section class=\"footnotes\">\n<hr>\n<ol>\n");
        let notes = std::mem::take(&mut r.footnotes);
        for (label, body) in notes {
            let _ = writeln!(r.out, "<li id=\"fn-{}\">{}</li>", esc(&label), body.trim_end());
        }
        r.out.push_str("</ol>\n</section>\n");
    }
    r.out
}

struct R<'a> {
    out: String,
    refs: HashMap<String, String>,
    resolver: &'a dyn LinkResolver,
    footnotes: Vec<(String, String)>,
}

/// Heading text flattened to plain text, used for ids and outlines.
pub fn plain_text(tree: &Tree, id: NodeId, bt: &str, out: &mut String) {
    for (c, n) in tree.children(id) {
        match n.kind {
            NodeKind::Text | NodeKind::Autolink | NodeKind::Tag => out.push_str(n.span.of(bt)),
            NodeKind::Escaped => out.push_str(&n.span.of(bt)[1..]),
            NodeKind::Entity => out.push_str(&decode_entity(n.span.of(bt))),
            NodeKind::Code { content } | NodeKind::InlineMath { content, .. } => out.push_str(content.of(bt)),
            NodeKind::SoftBreak | NodeKind::HardBreak => out.push(' '),
            NodeKind::WikiLink { target, subpath, alias, .. } => {
                if !alias.is_empty() {
                    out.push_str(alias.of(bt));
                } else {
                    out.push_str(target.of(bt));
                    if !subpath.is_empty() {
                        out.push_str(" > ");
                        out.push_str(subpath.of(bt));
                    }
                }
            }
            NodeKind::Comment | NodeKind::Html | NodeKind::BlockId { .. } | NodeKind::FootnoteRef { .. } => {}
            _ => plain_text(tree, c, bt, out),
        }
    }
}

pub fn decode_entity(e: &str) -> String {
    let body = e.trim_start_matches('&').trim_end_matches(';');
    let c = match body {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        "copy" => Some('©'),
        "reg" => Some('®'),
        "hellip" => Some('…'),
        "mdash" => Some('—'),
        "ndash" => Some('–'),
        "larr" => Some('←'),
        "rarr" => Some('→'),
        _ => body.strip_prefix('#').and_then(|n| {
            let code = match n.strip_prefix(['x', 'X']) {
                Some(h) => u32::from_str_radix(h, 16).ok(),
                None => n.parse().ok(),
            };
            code.and_then(char::from_u32)
        }),
    };
    c.map_or_else(|| e.to_string(), |c| c.to_string())
}

impl R<'_> {
    fn children(&mut self, tree: &Tree, id: NodeId, bt: &str, tight: bool) {
        for (c, n) in tree.children(id) {
            if n.kind.is_block() {
                self.block(tree, c, bt, tight);
            } else {
                self.inline(tree, c, bt);
            }
        }
    }

    fn inlines(&mut self, tree: &Tree, id: NodeId, bt: &str) {
        for (c, _) in tree.children(id) {
            self.inline(tree, c, bt);
        }
    }

    fn block(&mut self, tree: &Tree, id: NodeId, bt: &str, tight: bool) {
        let n = tree.node(id);
        match n.kind {
            NodeKind::Root
            | NodeKind::Blank
            | NodeKind::FrontMatter { .. }
            | NodeKind::LinkRefDef { .. }
            | NodeKind::CommentBlock
            | NodeKind::FootnoteDef { .. } => {}
            NodeKind::Heading { level, .. } => {
                let mut plain = String::new();
                plain_text(tree, id, bt, &mut plain);
                let _ = write!(self.out, "<h{level} data-heading=\"{}\">", esc(plain.trim()));
                self.inlines(tree, id, bt);
                let _ = writeln!(self.out, "</h{level}>");
            }
            NodeKind::Paragraph => {
                if tight {
                    self.inlines(tree, id, bt);
                    self.out.push('\n');
                } else {
                    self.out.push_str("<p>");
                    self.inlines(tree, id, bt);
                    self.out.push_str("</p>\n");
                }
            }
            NodeKind::CodeBlock { info, content, fence } => {
                let lang = info.of(bt).split_whitespace().next().unwrap_or("");
                if lang.is_empty() {
                    self.out.push_str("<pre><code>");
                } else {
                    let _ = write!(self.out, "<pre><code class=\"language-{}\">", esc(lang));
                }
                if fence == 0 {
                    for line in content.of(bt).lines() {
                        let (_, off) = crate::block::indent_of(line.as_bytes());
                        escape(&line[off.min(4)..], &mut self.out);
                        self.out.push('\n');
                    }
                } else if tree.node(id).first_child != crate::NO_NODE {
                    for (_, line) in tree.children(id) {
                        escape(line.span.of(bt), &mut self.out);
                    }
                } else {
                    escape(content.of(bt), &mut self.out);
                }
                self.out.push_str("</code></pre>\n");
            }
            NodeKind::MathBlock { content } => {
                self.out.push_str("<div class=\"math math-block\">");
                escape(content.of(bt).trim(), &mut self.out);
                self.out.push_str("</div>\n");
            }
            NodeKind::BlockQuote => {
                self.out.push_str("<blockquote>\n");
                self.children(tree, id, bt, false);
                self.out.push_str("</blockquote>\n");
            }
            NodeKind::Callout { kind, fold, title } => {
                let k = kind.of(bt).to_lowercase();
                let fold_attr = match fold {
                    Fold::None => "",
                    Fold::Open => " data-callout-fold=\"+\"",
                    Fold::Closed => " data-callout-fold=\"-\"",
                };
                let collapsed = if fold == Fold::Closed { " is-collapsed" } else { "" };
                let _ = write!(
                    self.out,
                    "<div class=\"callout{collapsed}\" data-callout=\"{}\"{fold_attr}>\n<div class=\"callout-title\">",
                    esc(&k)
                );
                let mut kids = tree.children(id).peekable();
                if title.is_empty() {
                    let mut c = k.chars();
                    let t: String = c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default();
                    escape(&t, &mut self.out);
                } else if let Some((tid, _)) = kids.next() {
                    self.inlines(tree, tid, bt);
                }
                self.out.push_str("</div>\n<div class=\"callout-content\">\n");
                for (c, _) in kids {
                    self.block(tree, c, bt, false);
                }
                self.out.push_str("</div>\n</div>\n");
            }
            NodeKind::List { ordered, start, tight } => {
                let has_task = tree
                    .children(id)
                    .any(|(_, n)| matches!(n.kind, NodeKind::ListItem { task: Some(_), .. }));
                let class = if has_task { " class=\"contains-task-list\"" } else { "" };
                if ordered && start != 1 {
                    let _ = writeln!(self.out, "<ol start=\"{start}\"{class}>");
                } else {
                    let _ = writeln!(self.out, "<{}{class}>", if ordered { "ol" } else { "ul" });
                }
                for (c, _) in tree.children(id) {
                    self.block(tree, c, bt, tight);
                }
                let _ = writeln!(self.out, "</{}>", if ordered { "ol" } else { "ul" });
            }
            NodeKind::ListItem { task, .. } => {
                match task {
                    Some(t) => {
                        let checked = if t == b' ' { "" } else { " checked" };
                        let done = if t == b' ' { "" } else { " is-checked" };
                        let _ = write!(
                            self.out,
                            "<li class=\"task-list-item{done}\" data-task=\"{}\"><input type=\"checkbox\" class=\"task-list-item-checkbox\"{checked}> ",
                            esc(&(t as char).to_string())
                        );
                    }
                    None => self.out.push_str("<li>"),
                }
                self.children(tree, id, bt, tight);
                if self.out.ends_with('\n') && tight {
                    self.out.pop();
                }
                self.out.push_str("</li>\n");
            }
            NodeKind::Table { .. } => {
                self.out.push_str("<table>\n");
                let mut in_body = false;
                for (rid, row) in tree.children(id) {
                    let header = matches!(row.kind, NodeKind::TableRow { header: true });
                    if header {
                        self.out.push_str("<thead>\n");
                    } else if !in_body {
                        self.out.push_str("<tbody>\n");
                        in_body = true;
                    }
                    self.out.push_str("<tr>");
                    for (cid, cell) in tree.children(rid) {
                        let tag = if header { "th" } else { "td" };
                        let align = match cell.kind {
                            NodeKind::TableCell { align: Align::Left } => " style=\"text-align:left\"",
                            NodeKind::TableCell { align: Align::Center } => " style=\"text-align:center\"",
                            NodeKind::TableCell { align: Align::Right } => " style=\"text-align:right\"",
                            _ => "",
                        };
                        let _ = write!(self.out, "<{tag}{align}>");
                        self.inlines(tree, cid, bt);
                        let _ = write!(self.out, "</{tag}>");
                    }
                    self.out.push_str("</tr>\n");
                    if header {
                        self.out.push_str("</thead>\n");
                    }
                }
                if in_body {
                    self.out.push_str("</tbody>\n");
                }
                self.out.push_str("</table>\n");
            }
            NodeKind::TableRow { .. } | NodeKind::TableCell { .. } => {}
            NodeKind::ThematicBreak => self.out.push_str("<hr>\n"),
            NodeKind::HtmlBlock => {
                self.out.push_str(n.span.of(bt));
                self.out.push('\n');
            }
            _ => self.inline(tree, id, bt),
        }
    }

    fn wrap(&mut self, tag: &str, tree: &Tree, id: NodeId, bt: &str) {
        let _ = write!(self.out, "<{tag}>");
        self.inlines(tree, id, bt);
        let _ = write!(self.out, "</{tag}>");
    }

    fn inline(&mut self, tree: &Tree, id: NodeId, bt: &str) {
        let n = tree.node(id);
        let src = n.span.of(bt);
        match n.kind {
            NodeKind::Text => escape(src, &mut self.out),
            NodeKind::SoftBreak => self.out.push('\n'),
            NodeKind::HardBreak => self.out.push_str("<br>\n"),
            NodeKind::Emph => self.wrap("em", tree, id, bt),
            NodeKind::Strong => self.wrap("strong", tree, id, bt),
            NodeKind::Strike => self.wrap("del", tree, id, bt),
            NodeKind::Highlight => self.wrap("mark", tree, id, bt),
            NodeKind::Code { content } => {
                self.out.push_str("<code>");
                let c = content.of(bt);
                let c = if c.len() > 2 && c.starts_with(' ') && c.ends_with(' ') && c.trim() != "" {
                    &c[1..c.len() - 1]
                } else {
                    c
                };
                escape(&c.replace('\n', " "), &mut self.out);
                self.out.push_str("</code>");
            }
            NodeKind::InlineMath { content, display } => {
                let class = if display { "math math-block" } else { "math math-inline" };
                let _ = write!(self.out, "<span class=\"{class}\">{}</span>", esc(content.of(bt)));
            }
            NodeKind::Link { dest, title } => {
                let _ = write!(self.out, "<a href=\"{}\"", esc(dest.of(bt)));
                if !title.is_empty() {
                    let _ = write!(self.out, " title=\"{}\"", esc(title.of(bt)));
                }
                let external = dest.of(bt).contains("://");
                self.out.push_str(if external { " class=\"external-link\">" } else { ">" });
                self.inlines(tree, id, bt);
                self.out.push_str("</a>");
            }
            NodeKind::Image { dest, title } => {
                let mut alt = String::new();
                plain_text(tree, id, bt, &mut alt);
                let _ = write!(self.out, "<img src=\"{}\" alt=\"{}\"", esc(dest.of(bt)), esc(&alt));
                if !title.is_empty() {
                    let _ = write!(self.out, " title=\"{}\"", esc(title.of(bt)));
                }
                self.out.push('>');
            }
            NodeKind::RefLink { label } => match self.refs.get(&label.of(bt).to_lowercase()).cloned() {
                Some(dest) => {
                    let _ = write!(self.out, "<a href=\"{}\">", esc(&dest));
                    self.inlines(tree, id, bt);
                    self.out.push_str("</a>");
                }
                None => escape(src, &mut self.out),
            },
            NodeKind::WikiLink { target, subpath, alias, embed } => {
                let t = target.of(bt).trim();
                let sub = subpath.of(bt);
                let full = if sub.is_empty() { t.to_string() } else { format!("{t}#{sub}") };
                if embed {
                    let _ = write!(self.out, "<span class=\"internal-embed\" src=\"{}\"", esc(&full));
                    if !alias.is_empty() {
                        let _ = write!(self.out, " alt=\"{}\"", esc(alias.of(bt)));
                    }
                    self.out.push_str("></span>");
                    return;
                }
                let resolved = if t.is_empty() { Some(String::new()) } else { self.resolver.resolve(t) };
                let class = if resolved.is_some() { "internal-link" } else { "internal-link is-unresolved" };
                let mut href = resolved.unwrap_or_else(|| t.to_string());
                if !sub.is_empty() {
                    href.push('#');
                    href.push_str(sub);
                }
                let _ = write!(
                    self.out,
                    "<a class=\"{class}\" data-href=\"{}\" href=\"{}\">",
                    esc(&full),
                    esc(&href)
                );
                if alias.is_empty() {
                    let shown = if t.is_empty() { sub.to_string() } else { full.replace('#', " > ") };
                    escape(&shown, &mut self.out);
                } else {
                    escape(alias.of(bt), &mut self.out);
                }
                self.out.push_str("</a>");
            }
            NodeKind::Tag => {
                let _ = write!(self.out, "<a class=\"tag\" href=\"{0}\">{0}</a>", esc(src));
            }
            NodeKind::FootnoteRef { label } => {
                let l = esc(label.of(bt));
                let _ = write!(self.out, "<sup class=\"footnote-ref\"><a href=\"#fn-{l}\">[{l}]</a></sup>");
            }
            NodeKind::InlineFootnote => {
                let mut body = String::new();
                std::mem::swap(&mut body, &mut self.out);
                self.inlines(tree, id, bt);
                std::mem::swap(&mut body, &mut self.out);
                let label = format!("inline-{}", self.footnotes.len() + 1);
                let _ = write!(
                    self.out,
                    "<sup class=\"footnote-ref\"><a href=\"#fn-{label}\">[{label}]</a></sup>"
                );
                self.footnotes.push((label, body));
            }
            NodeKind::Comment | NodeKind::BlockId { .. } => {}
            NodeKind::Html => self.out.push_str(src),
            NodeKind::Autolink => {
                let url = src.trim_start_matches('<').trim_end_matches('>');
                let href = if url.starts_with("www.") {
                    format!("http://{url}")
                } else if !url.contains(':') && url.contains('@') {
                    format!("mailto:{url}")
                } else {
                    url.to_string()
                };
                let _ =
                    write!(self.out, "<a href=\"{}\" class=\"external-link\">{}</a>", esc(&href), esc(url));
            }
            NodeKind::Escaped => escape(&src[1..], &mut self.out),
            NodeKind::Entity => escape(&decode_entity(src), &mut self.out),
            _ => {}
        }
    }
}
