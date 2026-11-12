//! Per-note metadata extracted from the parse tree: the equivalent of Obsidian's `CachedMetadata`.

use crate::path::percent_decode;
use md_engine::frontmatter::{self, Properties, Value};
use md_engine::{html, Document, NodeId, NodeKind, Tree};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LinkKind {
    Wiki,
    Markdown,
    /// A `[[link]]` inside a front matter value.
    FrontMatter,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Link {
    /// Target path as written (percent-decoded for Markdown links), without the subpath.
    pub target: Box<str>,
    /// Heading or `^block` reference, without the leading `#`.
    pub subpath: Box<str>,
    /// Alias or link text. Empty when the link shows its own target.
    pub display: Box<str>,
    /// Byte range of the whole link in the file.
    pub start: u32,
    pub end: u32,
    /// Byte range of the target path as written.
    pub target_start: u32,
    pub target_end: u32,
    pub embed: bool,
    pub kind: LinkKind,
}

impl Link {
    /// The text the link is displayed as.
    pub fn display_text(&self) -> String {
        if !self.display.is_empty() {
            self.display.to_string()
        } else if self.subpath.is_empty() {
            self.target.to_string()
        } else {
            format!("{}#{}", self.target, self.subpath)
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Heading {
    pub level: u8,
    pub text: Box<str>,
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TagRef {
    /// Without the leading `#`.
    pub tag: Box<str>,
    /// Byte offset of the `#`, or `u32::MAX` for a front matter tag.
    pub start: u32,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BlockRef {
    pub id: Box<str>,
    /// Range of the block the id is attached to.
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SectionKind {
    FrontMatter,
    Heading,
    Paragraph,
    Code,
    Math,
    Quote,
    Callout,
    List,
    Table,
    Rule,
    Html,
    Footnote,
    Comment,
    Definition,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Section {
    pub kind: SectionKind,
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ListItem {
    pub start: u32,
    pub end: u32,
    pub task: Option<u8>,
    /// Index of the parent item in `list_items`, or -1 for a top-level item.
    pub parent: i32,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct NoteMeta {
    pub links: Vec<Link>,
    pub tags: Vec<TagRef>,
    pub headings: Vec<Heading>,
    pub blocks: Vec<BlockRef>,
    pub sections: Vec<Section>,
    pub list_items: Vec<ListItem>,
    pub frontmatter: Option<Properties>,
    /// Set when the front matter exists but could not be parsed.
    pub frontmatter_error: Option<String>,
    /// End offset of the front matter block (0 if none).
    pub frontmatter_end: u32,
    pub aliases: Vec<Box<str>>,
    pub cssclasses: Vec<Box<str>>,
    pub words: u32,
}

impl NoteMeta {
    pub fn property(&self, key: &str) -> Option<&Value> {
        self.frontmatter.as_ref()?.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
    }
    /// All tags (inline and front matter), without `#`, deduplicated, in first-seen order.
    pub fn all_tags(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for t in &self.tags {
            if !out.iter().any(|x| x.eq_ignore_ascii_case(&t.tag)) {
                out.push(&t.tag);
            }
        }
        out
    }
    pub fn has_tag(&self, tag: &str) -> bool {
        let tag = tag.trim_start_matches('#');
        self.tags.iter().any(|t| tag_matches(&t.tag, tag))
    }
}

/// `tag:#a` matches `#a` and nested `#a/b`, case-insensitively.
pub fn tag_matches(have: &str, want: &str) -> bool {
    have.len() >= want.len()
        && have[..want.len()].eq_ignore_ascii_case(want)
        && (have.len() == want.len() || have.as_bytes()[want.len()] == b'/')
}

pub fn extract(text: &str) -> NoteMeta {
    extract_from(&md_engine::parse(text), text)
}

pub fn extract_from(doc: &Document, text: &str) -> NoteMeta {
    let mut m = NoteMeta::default();
    for b in &doc.blocks {
        let bt = b.text(text);
        let base = b.start as u32;
        let root = b.tree.root();
        let (rs, re) = (base + root.span.start, base + root.span.end);
        let kind = match root.kind {
            NodeKind::FrontMatter { content } => {
                m.frontmatter_end = re;
                match frontmatter::parse(content.of(bt)) {
                    Ok(p) => {
                        front_matter_derived(&mut m, &p, content.of(bt), base + content.start);
                        m.frontmatter = Some(p);
                    }
                    Err(e) => m.frontmatter_error = Some(e.to_string()),
                }
                Some(SectionKind::FrontMatter)
            }
            NodeKind::Heading { .. } => Some(SectionKind::Heading),
            NodeKind::Paragraph => Some(SectionKind::Paragraph),
            NodeKind::CodeBlock { .. } => Some(SectionKind::Code),
            NodeKind::MathBlock { .. } => Some(SectionKind::Math),
            NodeKind::BlockQuote => Some(SectionKind::Quote),
            NodeKind::Callout { .. } => Some(SectionKind::Callout),
            NodeKind::List { .. } => Some(SectionKind::List),
            NodeKind::Table { .. } => Some(SectionKind::Table),
            NodeKind::ThematicBreak => Some(SectionKind::Rule),
            NodeKind::HtmlBlock => Some(SectionKind::Html),
            NodeKind::FootnoteDef { .. } => Some(SectionKind::Footnote),
            NodeKind::CommentBlock => Some(SectionKind::Comment),
            NodeKind::LinkRefDef { .. } => Some(SectionKind::Definition),
            _ => None,
        };
        if let Some(kind) = kind {
            m.sections.push(Section { kind, start: rs, end: re });
        }
        walk(&b.tree, 0, bt, base, -1, &mut m);
        // A paragraph that is only `^id` labels the block before it.
        if let Some(last) = m.blocks.last_mut() {
            if last.start == rs
                && bt[root.span.range()].trim_start().starts_with('^')
                && m.sections.len() >= 2
            {
                let prev = m.sections[m.sections.len() - 2];
                last.start = prev.start;
                last.end = prev.end;
            }
        }
    }
    m
}

fn front_matter_derived(m: &mut NoteMeta, p: &Properties, src: &str, src_start: u32) {
    for (k, v) in p {
        let key = k.to_ascii_lowercase();
        match key.as_str() {
            "tags" | "tag" => {
                for t in v.as_string_list() {
                    for t in t.split_whitespace() {
                        let t = t.trim_start_matches('#');
                        if !t.is_empty() {
                            m.tags.push(TagRef { tag: t.into(), start: u32::MAX });
                        }
                    }
                }
            }
            "aliases" | "alias" => m.aliases.extend(v.as_string_list().into_iter().map(Into::into)),
            "cssclasses" | "cssclass" => {
                for c in v.as_string_list() {
                    m.cssclasses.extend(c.split_whitespace().map(Into::into));
                }
            }
            _ => {}
        }
    }
    // `[[links]]` inside property values.
    let mut i = 0;
    while let Some(p) = src[i..].find("[[") {
        let s = i + p;
        let Some(close) = src[s + 2..].find("]]").map(|c| s + 2 + c) else { break };
        let inner = &src[s + 2..close];
        i = close + 2;
        if inner.is_empty() || inner.contains('\n') || inner.contains("[[") {
            i = s + 2;
            continue;
        }
        let (link, alias) = inner.split_once('|').unwrap_or((inner, ""));
        let (target, subpath) = link.split_once('#').unwrap_or((link, ""));
        let ts = src_start + s as u32 + 2;
        m.links.push(Link {
            target: target.trim().into(),
            subpath: subpath.into(),
            display: alias.into(),
            start: src_start + s as u32,
            end: src_start + close as u32 + 2,
            target_start: ts,
            target_end: ts + target.len() as u32,
            embed: false,
            kind: LinkKind::FrontMatter,
        });
    }
}

fn is_external(dest: &str) -> bool {
    dest.contains("://")
        || dest.starts_with("mailto:")
        || dest.starts_with("tel:")
        || dest.starts_with("data:")
}

fn count_words(s: &str) -> u32 {
    let mut n = 0;
    let mut in_word = false;
    for c in s.chars() {
        // CJK characters count as one word each.
        let cjk = matches!(c as u32, 0x3040..=0x30ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xac00..=0xd7af);
        if cjk {
            n += 1;
            in_word = false;
        } else if c.is_alphanumeric() {
            if !in_word {
                n += 1;
                in_word = true;
            }
        } else if !matches!(c, '\'' | '’' | '-') {
            in_word = false;
        }
    }
    n
}

fn walk(tree: &Tree, id: NodeId, bt: &str, base: u32, parent_item: i32, m: &mut NoteMeta) {
    let n = tree.node(id);
    let (s, e) = (base + n.span.start, base + n.span.end);
    let mut item = parent_item;
    match n.kind {
        NodeKind::Heading { level, .. } if level > 0 => {
            let mut text = String::new();
            html::plain_text(tree, id, bt, &mut text);
            m.headings.push(Heading { level, text: text.trim().into(), start: s, end: e });
        }
        NodeKind::ListItem { task, .. } => {
            m.list_items.push(ListItem { start: s, end: e, task, parent: parent_item });
            item = m.list_items.len() as i32 - 1;
        }
        NodeKind::Text => m.words += count_words(n.span.of(bt)),
        NodeKind::Tag => m.tags.push(TagRef { tag: n.span.of(bt)[1..].into(), start: s }),
        NodeKind::WikiLink { target, subpath, alias, embed } => {
            let t = target.of(bt);
            // Without an alias the display text is derivable from target and subpath; it is not
            // stored (see `Link::display_text`), which saves one allocation per link.
            let shown = alias.of(bt);
            m.links.push(Link {
                target: t.trim().into(),
                subpath: subpath.of(bt).into(),
                display: shown.into(),
                start: s,
                end: e,
                target_start: base + target.start,
                target_end: base + target.end,
                embed,
                kind: LinkKind::Wiki,
            });
        }
        NodeKind::Link { dest, .. } | NodeKind::Image { dest, .. } => {
            let d = dest.of(bt);
            if !d.is_empty() && !is_external(d) {
                let (path, sub) = d.split_once('#').unwrap_or((d, ""));
                let mut display = String::new();
                html::plain_text(tree, id, bt, &mut display);
                m.links.push(Link {
                    target: percent_decode(path).into(),
                    subpath: percent_decode(sub).into(),
                    display: display.into(),
                    start: s,
                    end: e,
                    target_start: base + dest.start,
                    target_end: base + dest.start + path.len() as u32,
                    embed: matches!(n.kind, NodeKind::Image { .. }),
                    kind: LinkKind::Markdown,
                });
            }
        }
        NodeKind::BlockId { id: bid } => {
            // The id belongs to the enclosing paragraph, or to the list item when the paragraph
            // is the item's first child.
            let (mut bs, mut be) = (s, e);
            if let Some(owner) = owner_block(tree, id) {
                let o = tree.node(owner);
                (bs, be) = (base + o.span.start, base + o.span.end);
            }
            m.blocks.push(BlockRef { id: bid.of(bt).into(), start: bs, end: be });
        }
        _ => {}
    }
    for (c, _) in tree.children(id) {
        walk(tree, c, bt, base, item, m);
    }
}

/// Finds the block that a `BlockId` node labels by scanning for its parent chain.
fn owner_block(tree: &Tree, target: NodeId) -> Option<NodeId> {
    fn find(tree: &Tree, id: NodeId, target: NodeId, chain: &mut Vec<NodeId>) -> bool {
        if id == target {
            return true;
        }
        chain.push(id);
        for (c, _) in tree.children(id) {
            if c <= target && find(tree, c, target, chain) {
                return true;
            }
        }
        chain.pop();
        false
    }
    let mut chain = Vec::new();
    if !find(tree, 0, target, &mut chain) {
        return None;
    }
    let para = *chain.last()?;
    if chain.len() >= 2 {
        let parent = chain[chain.len() - 2];
        if matches!(tree.node(parent).kind, NodeKind::ListItem { .. })
            && tree.node(parent).first_child == para
        {
            return Some(parent);
        }
    }
    Some(para)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_everything() {
        let src = "---\ntags: [proj, area/work]\naliases: Alt Name\ncssclasses: wide\nup: \"[[Parent]]\"\n---\n# Title *x*\n\nText [[A]] and [[b/C#Sec|shown]] ![[img.png]] [md](My%20Note.md#H) [ext](https://x.y) #inline ^p1\n\n- [ ] task [[A]]\n  - [x] sub ^li\n\n## Second\n\n| t |\n|---|\n| 1 |\n\n^tbl\n";
        let m = extract(src);
        let targets: Vec<(&str, &str, bool)> =
            m.links.iter().map(|l| (&*l.target, &*l.subpath, l.embed)).collect();
        assert_eq!(
            targets,
            vec![
                ("Parent", "", false),
                ("A", "", false),
                ("b/C", "Sec", false),
                ("img.png", "", true),
                ("My Note.md", "H", false),
                ("A", "", false)
            ]
        );
        for l in &m.links {
            let written = &src[l.target_start as usize..l.target_end as usize];
            assert_eq!(percent_decode(written).trim(), &*l.target);
            assert!(src[l.start as usize..l.end as usize].contains(written));
        }
        assert_eq!(m.all_tags(), vec!["proj", "area/work", "inline"]);
        assert!(m.has_tag("#area") && m.has_tag("AREA/work") && !m.has_tag("are"));
        assert_eq!(m.aliases, vec!["Alt Name".into()]);
        assert_eq!(m.cssclasses, vec!["wide".into()]);
        let heads: Vec<(u8, &str)> = m.headings.iter().map(|h| (h.level, &*h.text)).collect();
        assert_eq!(heads, vec![(1, "Title x"), (2, "Second")]);
        let ids: Vec<&str> = m.blocks.iter().map(|b| &*b.id).collect();
        assert_eq!(ids, vec!["p1", "li", "tbl"]);
        assert!(src[m.blocks[0].start as usize..m.blocks[0].end as usize].starts_with("Text [[A]]"));
        assert!(src[m.blocks[1].start as usize..m.blocks[1].end as usize].contains("[x] sub"));
        assert!(src[m.blocks[2].start as usize..m.blocks[2].end as usize].starts_with("| t |"));
        assert_eq!(m.list_items.len(), 2);
        assert_eq!((m.list_items[0].task, m.list_items[0].parent), (Some(b' '), -1));
        assert_eq!((m.list_items[1].task, m.list_items[1].parent), (Some(b'x'), 0));
        assert_eq!(m.sections[0].kind, SectionKind::FrontMatter);
        assert_eq!(m.property("UP").and_then(|v| v.as_str()), Some("[[Parent]]"));
        assert!(m.words >= 8);
    }

    #[test]
    fn bad_front_matter_is_reported_not_fatal() {
        let m = extract("---\n: nope\n  bad\n---\nbody [[x]]\n");
        assert!(m.frontmatter.is_none() && m.frontmatter_error.is_some());
        assert_eq!(m.links.len(), 1);
    }
}
