//! Obsidian-flavored Markdown engine.
//!
//! The document is a sequence of top-level blocks that tile the source text. Each top-level block
//! owns a flat node arena whose spans are relative to the block start, so an edit only needs to
//! reparse the blocks around it and shift the start offset of the ones after it.

mod block;
mod doc;
pub mod frontmatter;
pub mod html;
mod inline;

pub use doc::{Document, Edit, TextSource, TopBlock};

/// Byte range. In a [`Tree`] it is relative to the start of the owning top-level block.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub const EMPTY: Span = Span { start: 0, end: 0 };

    pub fn new(start: usize, end: usize) -> Span {
        Span { start: start as u32, end: end as u32 }
    }
    pub fn len(self) -> usize {
        (self.end - self.start) as usize
    }
    pub fn is_empty(self) -> bool {
        self.end <= self.start
    }
    pub fn range(self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }
    /// The text this span covers, given the text of the owning top-level block.
    pub fn of(self, block_text: &str) -> &str {
        block_text.get(self.range()).unwrap_or("")
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fold {
    None,
    /// `[!note]+`
    Open,
    /// `[!note]-`
    Closed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NodeKind {
    // ---- blocks ----
    Root,
    Blank,
    FrontMatter {
        content: Span,
    },
    Heading {
        level: u8,
        setext: bool,
    },
    Paragraph,
    /// `fence` is the fence length, 0 for an indented code block.
    CodeBlock {
        fence: u8,
        info: Span,
        content: Span,
    },
    MathBlock {
        content: Span,
    },
    BlockQuote,
    Callout {
        kind: Span,
        fold: Fold,
        title: Span,
    },
    List {
        ordered: bool,
        start: u32,
        tight: bool,
    },
    /// `task` is the character between the brackets of `- [x]`.
    ListItem {
        task: Option<u8>,
        marker: Span,
    },
    Table {
        cols: u16,
    },
    TableRow {
        header: bool,
    },
    TableCell {
        align: Align,
    },
    ThematicBreak,
    HtmlBlock,
    FootnoteDef {
        label: Span,
    },
    LinkRefDef {
        label: Span,
        dest: Span,
        title: Span,
    },
    CommentBlock,
    // ---- inlines ----
    Text,
    SoftBreak,
    HardBreak,
    Emph,
    Strong,
    Strike,
    Highlight,
    Code {
        content: Span,
    },
    InlineMath {
        content: Span,
        display: bool,
    },
    Link {
        dest: Span,
        title: Span,
    },
    Image {
        dest: Span,
        title: Span,
    },
    RefLink {
        label: Span,
    },
    WikiLink {
        target: Span,
        subpath: Span,
        alias: Span,
        embed: bool,
    },
    /// Span covers `#tag`, including the hash.
    Tag,
    FootnoteRef {
        label: Span,
    },
    InlineFootnote,
    Comment,
    Html,
    Autolink,
    /// A backslash escape. The span covers both characters.
    Escaped,
    Entity,
    /// `^block-id` at the end of a block.
    BlockId {
        id: Span,
    },
}

impl NodeKind {
    pub fn is_block(self) -> bool {
        use NodeKind::*;
        matches!(
            self,
            Root | Blank
                | FrontMatter { .. }
                | Heading { .. }
                | Paragraph
                | CodeBlock { .. }
                | MathBlock { .. }
                | BlockQuote
                | Callout { .. }
                | List { .. }
                | ListItem { .. }
                | Table { .. }
                | TableRow { .. }
                | TableCell { .. }
                | ThematicBreak
                | HtmlBlock
                | FootnoteDef { .. }
                | LinkRefDef { .. }
                | CommentBlock
        )
    }
}

pub type NodeId = u32;
pub const NO_NODE: NodeId = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Node {
    pub kind: NodeKind,
    pub span: Span,
    pub first_child: NodeId,
    pub next: NodeId,
}

/// Flat arena for the nodes of one top-level block. Node 0 is the block itself.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Tree {
    pub nodes: Vec<Node>,
    /// Last child of each node; only populated while the tree is being built.
    last: Vec<NodeId>,
}

impl Tree {
    pub(crate) fn push(&mut self, parent: NodeId, kind: NodeKind, span: Span) -> NodeId {
        let id = self.nodes.len() as NodeId;
        self.nodes.push(Node { kind, span, first_child: NO_NODE, next: NO_NODE });
        self.last.push(NO_NODE);
        if parent != NO_NODE {
            let prev = self.last[parent as usize];
            if prev == NO_NODE {
                self.nodes[parent as usize].first_child = id;
            } else {
                self.nodes[prev as usize].next = id;
            }
            self.last[parent as usize] = id;
        }
        id
    }

    pub(crate) fn finish(&mut self) {
        self.last = Vec::new();
        self.nodes.shrink_to_fit();
    }

    pub fn root(&self) -> &Node {
        &self.nodes[0]
    }
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }
    pub fn children(&self, id: NodeId) -> Children<'_> {
        Children { tree: self, next: self.nodes.get(id as usize).map_or(NO_NODE, |n| n.first_child) }
    }
    /// All nodes in document order (the arena is filled in pre-order).
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.nodes.iter().enumerate().map(|(i, n)| (i as NodeId, n))
    }
}

pub struct Children<'a> {
    tree: &'a Tree,
    next: NodeId,
}

impl<'a> Iterator for Children<'a> {
    type Item = (NodeId, &'a Node);
    fn next(&mut self) -> Option<Self::Item> {
        if self.next == NO_NODE {
            return None;
        }
        let id = self.next;
        let n = &self.tree.nodes[id as usize];
        self.next = n.next;
        Some((id, n))
    }
}

/// Syntax extensions. Every Obsidian extension can be switched off individually.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Options {
    pub front_matter: bool,
    pub wikilinks: bool,
    pub tags: bool,
    pub highlights: bool,
    pub comments: bool,
    pub math: bool,
    pub callouts: bool,
    pub footnotes: bool,
    pub tables: bool,
    pub strikethrough: bool,
    pub autolinks: bool,
    pub block_ids: bool,
    pub task_lists: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            front_matter: true,
            wikilinks: true,
            tags: true,
            highlights: true,
            comments: true,
            math: true,
            callouts: true,
            footnotes: true,
            tables: true,
            strikethrough: true,
            autolinks: true,
            block_ids: true,
            task_lists: true,
        }
    }
}

impl Options {
    /// Plain CommonMark + GFM, no Obsidian syntax.
    pub fn gfm() -> Self {
        Options {
            wikilinks: false,
            tags: false,
            highlights: false,
            comments: false,
            math: false,
            callouts: false,
            block_ids: false,
            ..Options::default()
        }
    }
}

/// Parse a whole document.
pub fn parse(text: &str) -> Document {
    Document::parse(text, Options::default())
}
