//! Document = tiling sequence of top-level blocks, with incremental re-parse after an edit.

use crate::block::{parse_top, split_lines};
use crate::{NodeKind, Options, Tree};
use std::borrow::Cow;

/// Text the parser can read in windows, so a rope does not have to be flattened for every edit.
pub trait TextSource {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Text starting at byte `start` (a char boundary), at least `min_len` bytes long unless the
    /// end of the text comes first.
    fn slice_from(&self, start: usize, min_len: usize) -> Cow<'_, str>;
}

impl TextSource for str {
    fn len(&self) -> usize {
        str::len(self)
    }
    fn slice_from(&self, start: usize, min_len: usize) -> Cow<'_, str> {
        let mut end = start.saturating_add(min_len).min(str::len(self));
        while !self.is_char_boundary(end) {
            end += 1;
        }
        Cow::Borrowed(&self[start..end])
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TopBlock {
    /// Byte offset of the block in the document.
    pub start: usize,
    /// Length including trailing blank lines; the next block starts at `start + len`.
    pub len: usize,
    pub tree: Tree,
}

impl TopBlock {
    pub fn kind(&self) -> NodeKind {
        self.tree.root().kind
    }
    pub fn end(&self) -> usize {
        self.start + self.len
    }
    /// The block's text, given the whole document.
    pub fn text<'a>(&self, doc_text: &'a str) -> &'a str {
        &doc_text[self.start..self.end()]
    }
}

/// A replacement of `start..old_end` by text that now ends at `new_end`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Edit {
    pub start: usize,
    pub old_end: usize,
    pub new_end: usize,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Document {
    pub blocks: Vec<TopBlock>,
    pub opts: Options,
    len: usize,
}

const FIRST_WINDOW: usize = 4096;

impl Document {
    pub fn parse(text: &str, opts: Options) -> Document {
        let mut doc = Document { blocks: Vec::new(), opts, len: text.len() };
        doc.blocks = parse_run(text, 0, opts, text.len().max(1), |_| false);
        doc
    }

    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Index of the top-level block containing `offset`.
    pub fn block_index_at(&self, offset: usize) -> Option<usize> {
        if self.blocks.is_empty() {
            return None;
        }
        Some(self.blocks.partition_point(|b| b.start <= offset).saturating_sub(1))
    }

    /// Re-parse after `edit` was applied to the text. `text` is the text after the edit.
    /// Returns the range of block indices that were replaced (in the new block list).
    pub fn edit<S: TextSource + ?Sized>(&mut self, text: &S, edit: Edit) -> std::ops::Range<usize> {
        let old_len = self.len;
        self.len = text.len();
        debug_assert_eq!(old_len + edit.new_end - edit.start, self.len + edit.old_end - edit.start);
        let Some(hit) = self.block_index_at(edit.start) else {
            self.blocks = parse_run(text, 0, self.opts, self.len.max(1), |_| false);
            return 0..self.blocks.len();
        };
        // An edit can change how the preceding block ends (lazy continuation, setext underline,
        // table delimiter row), so restart a little earlier.
        let mut first = hit.saturating_sub(2);
        // A front matter block needs its closing fence, which may be typed anywhere later. While
        // the document starts with an unclosed `---`, re-parse from the top.
        if matches!(self.blocks[0].kind(), NodeKind::ThematicBreak)
            && text.slice_from(0, 3).starts_with("---")
        {
            first = 0;
        }
        let from = self.blocks[first].start;
        let blocks = &self.blocks;
        let mut resume = None;
        let fresh = parse_run(text, from, self.opts, FIRST_WINDOW, |end| {
            if end < edit.new_end {
                return false;
            }
            let old = end + edit.old_end - edit.new_end;
            if old < edit.old_end {
                return false;
            }
            match blocks.binary_search_by_key(&old, |b| b.start) {
                Ok(j) if j > hit => {
                    resume = Some(j);
                    true
                }
                _ => false,
            }
        });
        let n = fresh.len();
        match resume {
            Some(j) => {
                if edit.new_end != edit.old_end {
                    for b in &mut self.blocks[j..] {
                        b.start = b.start + edit.new_end - edit.old_end;
                    }
                }
                self.blocks.splice(first..j, fresh);
            }
            None => {
                self.blocks.truncate(first);
                self.blocks.extend(fresh);
            }
        }
        first..first + n
    }

    /// Debug dump of the tree, used by snapshot tests.
    pub fn dump(&self, text: &str) -> String {
        let mut out = String::new();
        for b in &self.blocks {
            let bt = b.text(text);
            dump_node(&b.tree, 0, bt, 0, &mut out);
        }
        out
    }
}

fn dump_node(tree: &Tree, id: crate::NodeId, bt: &str, depth: usize, out: &mut String) {
    use std::fmt::Write;
    let n = tree.node(id);
    let _ = write!(out, "{:indent$}", "", indent = depth * 2);
    let name = format!("{:?}", n.kind);
    let name = name.split([' ', '{']).next().unwrap_or("");
    let _ = write!(out, "{name}");
    match n.kind {
        NodeKind::Heading { level, .. } => _ = write!(out, " h{level}"),
        NodeKind::CodeBlock { info, content, .. } => {
            _ = write!(out, " info={:?} {:?}", info.of(bt), content.of(bt))
        }
        NodeKind::MathBlock { content } | NodeKind::Code { content } | NodeKind::FrontMatter { content } => {
            _ = write!(out, " {:?}", content.of(bt))
        }
        NodeKind::InlineMath { content, display } => {
            _ = write!(out, " display={display} {:?}", content.of(bt))
        }
        NodeKind::Callout { kind, fold, title } => {
            _ = write!(out, " kind={:?} fold={fold:?} title={:?}", kind.of(bt), title.of(bt))
        }
        NodeKind::List { ordered, start, tight } => {
            _ = write!(out, " ordered={ordered} start={start} tight={tight}")
        }
        NodeKind::ListItem { task: Some(t), .. } => _ = write!(out, " task={:?}", t as char),
        NodeKind::TableCell { align } => _ = write!(out, " {align:?}"),
        NodeKind::TableRow { header } if header => _ = write!(out, " header"),
        NodeKind::FootnoteDef { label } | NodeKind::FootnoteRef { label } | NodeKind::RefLink { label } => {
            _ = write!(out, " label={:?}", label.of(bt))
        }
        NodeKind::LinkRefDef { label, dest, title } => {
            _ = write!(out, " label={:?} dest={:?} title={:?}", label.of(bt), dest.of(bt), title.of(bt))
        }
        NodeKind::Link { dest, title } | NodeKind::Image { dest, title } => {
            _ = write!(out, " dest={:?} title={:?}", dest.of(bt), title.of(bt))
        }
        NodeKind::WikiLink { target, subpath, alias, embed } => {
            _ = write!(
                out,
                " target={:?} subpath={:?} alias={:?} embed={embed}",
                target.of(bt),
                subpath.of(bt),
                alias.of(bt)
            )
        }
        NodeKind::BlockId { id } => _ = write!(out, " id={:?}", id.of(bt)),
        NodeKind::Text
        | NodeKind::Tag
        | NodeKind::Autolink
        | NodeKind::Html
        | NodeKind::Escaped
        | NodeKind::Entity
        | NodeKind::Comment
        | NodeKind::HtmlBlock
        | NodeKind::CommentBlock => _ = write!(out, " {:?}", n.span.of(bt)),
        _ => {}
    }
    out.push('\n');
    for (c, _) in tree.children(id) {
        dump_node(tree, c, bt, depth + 1, out);
    }
}

/// Parse top-level blocks starting at `from` until `stop(block_end)` returns true or the text ends.
fn parse_run<S: TextSource + ?Sized>(
    text: &S,
    from: usize,
    opts: Options,
    first_window: usize,
    mut stop: impl FnMut(usize) -> bool,
) -> Vec<TopBlock> {
    let total = text.len();
    let mut out = Vec::new();
    let mut pos = from;
    let mut want = first_window;
    while pos < total {
        let window = text.slice_from(pos, want);
        let at_eof = pos + window.len() >= total;
        let lines = split_lines(&window);
        let mut i = 0;
        let mut progressed = false;
        while i < lines.len() {
            let (tree, next) = parse_top(&window, &lines, i, opts, pos + lines[i].s == 0);
            // The parser looks at most one line past the block. If that line may be cut off by
            // the window, retry with a larger one.
            if !at_eof && next + 1 >= lines.len() {
                break;
            }
            let start = pos + lines[i].s;
            let end = pos + if next < lines.len() { lines[next].s } else { window.len() };
            out.push(TopBlock { start, len: end - start, tree });
            progressed = true;
            i = next;
            if stop(end) {
                return out;
            }
        }
        if i >= lines.len() {
            break;
        }
        pos += lines[i].s;
        if !progressed {
            want = want.saturating_mul(4);
        }
    }
    out
}
