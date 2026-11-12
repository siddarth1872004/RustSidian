//! Inline parser. Runs over the logical text of a leaf block (its lines joined with `\n`, container
//! prefixes removed) and maps spans back to source offsets when the nodes are emitted.

use crate::block::Parser;
use crate::{NodeId, NodeKind, Options, Span};

const MAX_DEPTH: u32 = 12;
/// Longest link text / wikilink / inline footnote that is searched for a closing bracket.
const BRACKET_WINDOW: usize = 4096;

struct T {
    kind: NodeKind,
    s: usize,
    e: usize,
    children: Vec<u32>,
}

#[derive(Clone, Copy)]
enum Item {
    Node(u32),
    Delim { ch: u8, n: usize, orig: usize, pos: usize, open: bool, close: bool },
}

struct Ctx<'t> {
    t: &'t str,
    b: &'t [u8],
    opts: Options,
    nodes: Vec<T>,
    depth: u32,
    /// Backtick run lengths known to have no closing run further on (bit n).
    no_tick: u64,
    no_dollar: bool,
    no_comment: bool,
}

pub(crate) fn parse(p: &mut Parser<'_>, parent: NodeId, segs: &[(usize, usize)]) {
    let joined;
    let (t, starts): (&str, Vec<usize>) = if segs.len() == 1 {
        (&p.text[segs[0].0..segs[0].1], vec![0])
    } else {
        let mut s = String::with_capacity(segs.iter().map(|x| x.1 - x.0 + 1).sum());
        let mut starts = Vec::with_capacity(segs.len());
        for (i, &(a, b)) in segs.iter().enumerate() {
            if i > 0 {
                s.push('\n');
            }
            starts.push(s.len());
            s.push_str(&p.text[a..b]);
        }
        joined = s;
        (&joined, starts)
    };
    if t.is_empty() {
        return;
    }
    let mut cx = Ctx {
        t,
        b: t.as_bytes(),
        opts: p.opts,
        nodes: Vec::new(),
        depth: 0,
        no_tick: 0,
        no_dollar: false,
        no_comment: false,
    };
    let mut end = t.len();
    let mut block_id = None;
    if p.opts.block_ids && matches!(p.tree.node(parent).kind, NodeKind::Paragraph) {
        if let Some((ws, caret)) = find_block_id(cx.b) {
            block_id = Some(caret);
            end = ws;
        }
    }
    let mut kids = cx.range(0, end);
    if let Some(caret) = block_id {
        let id = cx.node(NodeKind::BlockId { id: Span::new(caret + 1, t.len()) }, caret, t.len());
        kids.push(id);
    }
    // Emit into the block tree, mapping logical offsets to block-relative source offsets.
    let base = p.base;
    let map = |off: usize| -> u32 {
        let i = starts.partition_point(|&s| s <= off).saturating_sub(1);
        (segs[i].0 + (off - starts[i]).min(segs[i].1 - segs[i].0) - base) as u32
    };
    let ms = |sp: Span| Span { start: map(sp.start as usize), end: map(sp.end as usize) };
    let mut stack: Vec<(u32, NodeId)> = kids.iter().rev().map(|&k| (k, parent)).collect();
    while let Some((tid, par)) = stack.pop() {
        let n = &cx.nodes[tid as usize];
        let kind = map_kind(n.kind, &ms);
        let id = p.tree.push(par, kind, Span { start: map(n.s), end: map(n.e) });
        for &c in n.children.iter().rev() {
            stack.push((c, id));
        }
    }
}

fn map_kind(k: NodeKind, m: &impl Fn(Span) -> Span) -> NodeKind {
    use NodeKind::*;
    match k {
        Code { content } => Code { content: m(content) },
        InlineMath { content, display } => InlineMath { content: m(content), display },
        Link { dest, title } => Link { dest: m(dest), title: m(title) },
        Image { dest, title } => Image { dest: m(dest), title: m(title) },
        RefLink { label } => RefLink { label: m(label) },
        WikiLink { target, subpath, alias, embed } => {
            WikiLink { target: m(target), subpath: m(subpath), alias: m(alias), embed }
        }
        FootnoteRef { label } => FootnoteRef { label: m(label) },
        BlockId { id } => BlockId { id: m(id) },
        other => other,
    }
}

/// `text ^block-id` -> (end of text, offset of `^`)
fn find_block_id(b: &[u8]) -> Option<(usize, usize)> {
    let mut i = b.len();
    while i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'-') {
        i -= 1;
    }
    if i == b.len() || i == 0 || b[i - 1] != b'^' {
        return None;
    }
    let caret = i - 1;
    if caret == 0 {
        return Some((0, 0));
    }
    if !matches!(b[caret - 1], b' ' | b'\n' | b'\t') {
        return None;
    }
    let mut ws = caret;
    while ws > 0 && matches!(b[ws - 1], b' ' | b'\t') {
        ws -= 1;
    }
    Some((ws, caret))
}

fn is_punct(c: char) -> bool {
    c.is_ascii_punctuation() || (!c.is_ascii() && !c.is_alphanumeric() && !c.is_whitespace())
}

fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '/')
}

impl<'t> Ctx<'t> {
    fn node(&mut self, kind: NodeKind, s: usize, e: usize) -> u32 {
        self.nodes.push(T { kind, s, e, children: Vec::new() });
        (self.nodes.len() - 1) as u32
    }

    fn find(&self, from: usize, to: usize, needle: &[u8]) -> Option<usize> {
        if from >= to {
            return None;
        }
        self.b[from..to].windows(needle.len()).position(|w| w == needle).map(|p| from + p)
    }

    fn prev_char(&self, i: usize, s: usize) -> char {
        if i <= s {
            ' '
        } else {
            self.t[..i].chars().next_back().unwrap_or(' ')
        }
    }
    fn next_char(&self, i: usize, e: usize) -> char {
        if i >= e {
            ' '
        } else {
            self.t[i..].chars().next().unwrap_or(' ')
        }
    }

    /// Parse `s..e` into a list of child node ids.
    fn range(&mut self, s: usize, e: usize) -> Vec<u32> {
        let mut items: Vec<Item> = Vec::new();
        let mut text_start = s;
        let mut i = s;
        macro_rules! flush {
            ($upto:expr) => {
                if $upto > text_start {
                    let id = self.node(NodeKind::Text, text_start, $upto);
                    items.push(Item::Node(id));
                }
            };
        }
        macro_rules! emit {
            ($kind:expr, $end:expr) => {{
                flush!(i);
                let id = self.node($kind, i, $end);
                items.push(Item::Node(id));
                i = $end;
                text_start = i;
                continue;
            }};
        }
        while i < e {
            let c = self.b[i];
            match c {
                b'\\' if i + 1 < e => {
                    let n = self.b[i + 1];
                    if n.is_ascii_punctuation() {
                        emit!(NodeKind::Escaped, i + 2);
                    } else if n == b'\n' {
                        emit!(NodeKind::HardBreak, i + 2);
                    }
                }
                b'\n' => {
                    let hard = i >= s + 2 && self.b[i - 1] == b' ' && self.b[i - 2] == b' ';
                    emit!(if hard { NodeKind::HardBreak } else { NodeKind::SoftBreak }, i + 1);
                }
                b'`' => {
                    let n = self.b[i..e].iter().take_while(|&&x| x == b'`').count();
                    if let Some(close) = self.find_tick_run(i + n, e, n) {
                        emit!(NodeKind::Code { content: Span::new(i + n, close) }, close + n);
                    }
                    i += n;
                    continue;
                }
                b'$' if self.opts.math && !self.no_dollar => {
                    if self.b[i..e].starts_with(b"$$") {
                        if let Some(k) = self.find(i + 2, e, b"$$") {
                            let kind = NodeKind::InlineMath { content: Span::new(i + 2, k), display: true };
                            emit!(kind, k + 2);
                        }
                        i += 2;
                        continue;
                    }
                    if i + 1 < e && !self.b[i + 1].is_ascii_whitespace() {
                        let mut k = i + 1;
                        let mut found = None;
                        let mut any = false;
                        while k < e {
                            match self.b[k] {
                                b'\\' => k += 1,
                                b'$' if k + 1 < e && self.b[k + 1] == b'$' => {
                                    any = true;
                                    k += 1;
                                }
                                b'$' => {
                                    any = true;
                                    let before_ok = !self.b[k - 1].is_ascii_whitespace();
                                    let after_ok = k + 1 >= e || !self.b[k + 1].is_ascii_digit();
                                    if before_ok && after_ok && k > i + 1 {
                                        found = Some(k);
                                        break;
                                    }
                                }
                                _ => {}
                            }
                            k += 1;
                        }
                        if let Some(k) = found {
                            let kind = NodeKind::InlineMath { content: Span::new(i + 1, k), display: false };
                            emit!(kind, k + 1);
                        }
                        if !any {
                            self.no_dollar = true;
                        }
                    }
                }
                b'%' if self.opts.comments && !self.no_comment && self.b[i..e].starts_with(b"%%") => {
                    if let Some(k) = self.find(i + 2, e, b"%%") {
                        emit!(NodeKind::Comment, k + 2);
                    }
                    self.no_comment = true;
                    i += 2;
                    continue;
                }
                b'!' if i + 1 < e && self.b[i + 1] == b'[' => {
                    if self.opts.wikilinks && self.b[i + 1..e].starts_with(b"[[") {
                        if let Some((end, kind)) = self.wikilink(i + 1, e, true) {
                            emit!(kind, end);
                        }
                    }
                    if let Some((end, id)) = self.link(i + 1, e, true) {
                        flush!(i);
                        self.nodes[id as usize].s = i;
                        items.push(Item::Node(id));
                        i = end;
                        text_start = i;
                        continue;
                    }
                }
                b'[' => {
                    if self.opts.wikilinks && self.b[i..e].starts_with(b"[[") {
                        if let Some((end, kind)) = self.wikilink(i, e, false) {
                            emit!(kind, end);
                        }
                    }
                    if self.opts.footnotes && i + 1 < e && self.b[i + 1] == b'^' {
                        let lim = e.min(i + BRACKET_WINDOW);
                        if let Some(k) = self.b[i..lim].iter().position(|&x| x == b']').map(|p| i + p) {
                            if k > i + 2 && !self.b[i + 2..k].iter().any(|x| x.is_ascii_whitespace()) {
                                emit!(NodeKind::FootnoteRef { label: Span::new(i + 2, k) }, k + 1);
                            }
                        }
                    }
                    if let Some((end, id)) = self.link(i, e, false) {
                        flush!(i);
                        items.push(Item::Node(id));
                        i = end;
                        text_start = i;
                        continue;
                    }
                }
                b'^' if self.opts.footnotes
                    && i + 1 < e
                    && self.b[i + 1] == b'['
                    && self.depth < MAX_DEPTH =>
                {
                    if let Some(k) = self.close_bracket(i + 1, e) {
                        flush!(i);
                        self.depth += 1;
                        let kids = self.range(i + 2, k);
                        self.depth -= 1;
                        let id = self.node(NodeKind::InlineFootnote, i, k + 1);
                        self.nodes[id as usize].children = kids;
                        items.push(Item::Node(id));
                        i = k + 1;
                        text_start = i;
                        continue;
                    }
                }
                b'<' => {
                    if let Some(end) = self.angle(i, e) {
                        let inner = &self.t[i + 1..end - 1];
                        let auto = (inner.contains("://") || inner.starts_with("mailto:") || is_email(inner))
                            && !inner.contains(char::is_whitespace);
                        emit!(if auto { NodeKind::Autolink } else { NodeKind::Html }, end);
                    }
                }
                b'&' => {
                    let lim = e.min(i + 34);
                    if let Some(k) = self.b[i + 1..lim].iter().position(|&x| x == b';').map(|p| i + 1 + p) {
                        let body = &self.b[i + 1..k];
                        let ok = !body.is_empty()
                            && (body.iter().all(|x| x.is_ascii_alphanumeric())
                                || (body[0] == b'#'
                                    && body.len() > 1
                                    && body[1..].iter().all(|x| x.is_ascii_alphanumeric())));
                        if ok {
                            emit!(NodeKind::Entity, k + 1);
                        }
                    }
                }
                b'#' if self.opts.tags => {
                    let prev = self.prev_char(i, s);
                    let prev_ok = i == s
                        || prev.is_whitespace()
                        || (is_punct(prev) && !matches!(prev, '#' | '/' | '&' | '=' | '\\'));
                    if prev_ok {
                        let mut end = i + 1;
                        let mut non_digit = false;
                        for ch in self.t[i + 1..e].chars() {
                            if !is_tag_char(ch) {
                                break;
                            }
                            non_digit |= !ch.is_ascii_digit();
                            end += ch.len_utf8();
                        }
                        if end > i + 1 && non_digit {
                            emit!(NodeKind::Tag, end);
                        }
                    }
                }
                b'h' | b'w' if self.opts.autolinks => {
                    let rest = &self.b[i..e];
                    let is_url = rest.starts_with(b"http://")
                        || rest.starts_with(b"https://")
                        || rest.starts_with(b"www.");
                    if is_url && !self.prev_char(i, s).is_alphanumeric() {
                        let mut end =
                            i + rest.iter().take_while(|&&x| !x.is_ascii_whitespace() && x != b'<').count();
                        while end > i {
                            let last = self.b[end - 1];
                            let unbalanced = last == b')' && {
                                let seg = &self.b[i..end];
                                seg.iter().filter(|&&x| x == b')').count()
                                    > seg.iter().filter(|&&x| x == b'(').count()
                            };
                            if matches!(
                                last,
                                b'.' | b',' | b':' | b';' | b'!' | b'?' | b'\'' | b'"' | b'*' | b'_' | b'~'
                            ) || unbalanced
                            {
                                end -= 1;
                            } else {
                                break;
                            }
                        }
                        let min = if rest.starts_with(b"www.") { 5 } else { 8 };
                        if end - i >= min && self.t.is_char_boundary(end) {
                            emit!(NodeKind::Autolink, end);
                        }
                    }
                }
                b'*' | b'_' | b'~' | b'=' => {
                    let enabled = match c {
                        b'~' => self.opts.strikethrough,
                        b'=' => self.opts.highlights,
                        _ => true,
                    };
                    let n = self.b[i..e].iter().take_while(|&&x| x == c).count();
                    let usable = match c {
                        b'~' => n <= 2,
                        b'=' => n >= 2,
                        _ => true,
                    };
                    if enabled && usable {
                        let prev = self.prev_char(i, s);
                        let next = self.next_char(i + n, e);
                        let lf = !next.is_whitespace()
                            && (!is_punct(next) || prev.is_whitespace() || is_punct(prev));
                        let rf = !prev.is_whitespace()
                            && (!is_punct(prev) || next.is_whitespace() || is_punct(next));
                        let (open, close) = if c == b'_' {
                            (lf && (!rf || is_punct(prev)), rf && (!lf || is_punct(next)))
                        } else {
                            (lf, rf)
                        };
                        if open || close {
                            flush!(i);
                            items.push(Item::Delim { ch: c, n, orig: n, pos: i, open, close });
                            i += n;
                            text_start = i;
                            continue;
                        }
                    }
                    i += n;
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
        flush!(e);
        self.resolve(items)
    }

    fn find_tick_run(&mut self, from: usize, e: usize, n: usize) -> Option<usize> {
        if n < 64 && self.no_tick & (1 << n) != 0 {
            return None;
        }
        let mut k = from;
        while k < e {
            if self.b[k] == b'`' {
                let m = self.b[k..e].iter().take_while(|&&x| x == b'`').count();
                if m == n {
                    return Some(k);
                }
                k += m;
            } else {
                k += 1;
            }
        }
        if n < 64 {
            self.no_tick |= 1 << n;
        }
        None
    }

    /// `open` points at `[`. Returns the offset of the matching `]`.
    fn close_bracket(&self, open: usize, e: usize) -> Option<usize> {
        let lim = e.min(open + BRACKET_WINDOW);
        let mut depth = 0;
        let mut k = open;
        while k < lim {
            match self.b[k] {
                b'\\' => k += 1,
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(k);
                    }
                }
                _ => {}
            }
            k += 1;
        }
        None
    }

    fn wikilink(&self, i: usize, e: usize, embed: bool) -> Option<(usize, NodeKind)> {
        let lim = e.min(i + BRACKET_WINDOW);
        let mut k = i + 2;
        while k + 1 < lim {
            match self.b[k] {
                b'\n' => return None,
                b'[' if self.b[k + 1] == b'[' => return None,
                b']' if self.b[k + 1] == b']' => break,
                _ => k += 1,
            }
        }
        if k + 1 >= lim || k == i + 2 || self.b[k] != b']' {
            return None;
        }
        let inner = &self.b[i + 2..k];
        let (mut link_end, alias) = match inner.iter().position(|&x| x == b'|') {
            Some(p) => (i + 2 + p, Span::new(i + 2 + p + 1, k)),
            None => (k, Span::EMPTY),
        };
        // `[[a\|b]]` inside tables: the backslash is not part of the target.
        if !alias.is_empty() && link_end > i + 2 && self.b[link_end - 1] == b'\\' {
            link_end -= 1;
        }
        let (target, subpath) = match self.b[i + 2..link_end].iter().position(|&x| x == b'#') {
            Some(p) => (Span::new(i + 2, i + 2 + p), Span::new(i + 2 + p + 1, link_end)),
            None => (Span::new(i + 2, link_end), Span::EMPTY),
        };
        Some((k + 2, NodeKind::WikiLink { target, subpath, alias, embed }))
    }

    /// `i` points at `[`. Returns (end offset, node id).
    fn link(&mut self, i: usize, e: usize, image: bool) -> Option<(usize, u32)> {
        if self.depth >= MAX_DEPTH {
            return None;
        }
        let k = self.close_bracket(i, e)?;
        let after = k + 1;
        let (end, kind) = if after < e && self.b[after] == b'(' {
            let mut p = after + 1;
            while p < e && matches!(self.b[p], b' ' | b'\n') {
                p += 1;
            }
            let (ds, de);
            if p < e && self.b[p] == b'<' {
                let close = self.b[p..e].iter().position(|&x| x == b'>' || x == b'\n').map(|q| p + q)?;
                if self.b[close] != b'>' {
                    return None;
                }
                (ds, de) = (p + 1, close);
                p = close + 1;
            } else {
                let mut depth = 0;
                ds = p;
                while p < e {
                    match self.b[p] {
                        b'\\' => p += 1,
                        b'(' => depth += 1,
                        b')' if depth == 0 => break,
                        b')' => depth -= 1,
                        b' ' | b'\n' | b'\t' => break,
                        _ => {}
                    }
                    p += 1;
                }
                de = p.min(e);
            }
            while p < e && matches!(self.b[p], b' ' | b'\n' | b'\t') {
                p += 1;
            }
            let mut title = Span::EMPTY;
            if p < e && matches!(self.b[p], b'"' | b'\'') {
                let q = self.b[p];
                let close = self.b[p + 1..e].iter().position(|&x| x == q).map(|x| p + 1 + x)?;
                title = Span::new(p + 1, close);
                p = close + 1;
                while p < e && matches!(self.b[p], b' ' | b'\n' | b'\t') {
                    p += 1;
                }
            }
            if p >= e || self.b[p] != b')' {
                return None;
            }
            let dest = Span::new(ds, de);
            (p + 1, if image { NodeKind::Image { dest, title } } else { NodeKind::Link { dest, title } })
        } else if after < e && self.b[after] == b'[' && !image {
            let close =
                self.b[after..e.min(after + 1000)].iter().position(|&x| x == b']').map(|q| after + q)?;
            let label = if close == after + 1 { Span::new(i + 1, k) } else { Span::new(after + 1, close) };
            (close + 1, NodeKind::RefLink { label })
        } else {
            return None;
        };
        self.depth += 1;
        let kids = self.range(i + 1, k);
        self.depth -= 1;
        let id = self.node(kind, i, end);
        self.nodes[id as usize].children = kids;
        Some((end, id))
    }

    /// `<...>`: returns the end offset if this looks like an autolink or an HTML tag/comment.
    fn angle(&self, i: usize, e: usize) -> Option<usize> {
        let rest = &self.b[i..e];
        if rest.starts_with(b"<!--") {
            return self.find(i + 4, e, b"-->").map(|k| k + 3);
        }
        let first = *rest.get(1)?;
        if !(first.is_ascii_alphabetic() || first == b'/') {
            return None;
        }
        let lim = e.min(i + BRACKET_WINDOW);
        let close = self.b[i..lim].iter().position(|&x| x == b'>').map(|p| i + p)?;
        if self.b[i + 1..close].contains(&b'<') {
            return None;
        }
        Some(close + 1)
    }

    fn resolve(&mut self, mut items: Vec<Item>) -> Vec<u32> {
        // openers_bottom, keyed by (delimiter, closer can open, closer length % 3)
        let mut bottom = [0usize; 24];
        let key = |ch: u8, open: bool, orig: usize| -> usize {
            let c = match ch {
                b'*' => 0,
                b'_' => 1,
                b'~' => 2,
                _ => 3,
            };
            c * 6 + (open as usize) * 3 + orig % 3
        };
        let mut ci = 0;
        while ci < items.len() {
            let Item::Delim { ch, n, orig, pos, open, close } = items[ci] else {
                ci += 1;
                continue;
            };
            if !close || n == 0 {
                ci += 1;
                continue;
            }
            let kb = key(ch, open, orig);
            let mut found = None;
            let mut oi = ci;
            while oi > bottom[kb] {
                oi -= 1;
                if let Item::Delim { ch: c2, n: n2, orig: o2, open: op2, close: cl2, .. } = items[oi] {
                    if c2 != ch || !op2 || n2 == 0 {
                        continue;
                    }
                    if matches!(ch, b'*' | b'_')
                        && (open || cl2)
                        && (o2 + orig) % 3 == 0
                        && !(o2 % 3 == 0 && orig % 3 == 0)
                    {
                        continue;
                    }
                    if ch == b'~' && n2 != n {
                        continue;
                    }
                    if ch == b'=' && (n2 < 2 || n < 2) {
                        continue;
                    }
                    found = Some(oi);
                    break;
                }
            }
            let Some(oi) = found else {
                bottom[kb] = ci;
                ci += 1;
                continue;
            };
            let Item::Delim { n: on, pos: opos, .. } = items[oi] else { unreachable!() };
            let (used, kind) = match ch {
                b'~' => (n, NodeKind::Strike),
                b'=' => (2, NodeKind::Highlight),
                _ if n >= 2 && on >= 2 => (2, NodeKind::Strong),
                _ => (1, NodeKind::Emph),
            };
            let kids = self.collect_nodes(&items[oi + 1..ci]);
            let id = self.node(kind, opos + on - used, pos + used);
            self.nodes[id as usize].children = kids;
            items.splice(oi + 1..ci, [Item::Node(id)]);
            for b in bottom.iter_mut() {
                *b = (*b).min(oi + 1);
            }
            // Indices now: opener at oi, new node at oi+1, closer at oi+2.
            let mut closer_idx = oi + 2;
            if let Item::Delim { n, .. } = &mut items[closer_idx] {
                *n -= used;
            }
            if let Item::Delim { pos, .. } = &mut items[closer_idx] {
                *pos += used;
            }
            if let Item::Delim { n, .. } = &mut items[oi] {
                *n -= used;
                if *n == 0 {
                    items.remove(oi);
                    closer_idx -= 1;
                    for b in bottom.iter_mut() {
                        *b = (*b).min(oi);
                    }
                }
            }
            if matches!(items[closer_idx], Item::Delim { n: 0, .. }) {
                items.remove(closer_idx);
            }
            ci = closer_idx;
        }
        self.collect_nodes(&items)
    }

    fn collect_nodes(&mut self, items: &[Item]) -> Vec<u32> {
        let mut out: Vec<u32> = Vec::with_capacity(items.len());
        for it in items {
            let id = match *it {
                Item::Node(id) => id,
                Item::Delim { n: 0, .. } => continue,
                Item::Delim { n, pos, .. } => self.node(NodeKind::Text, pos, pos + n),
            };
            // Merge adjacent text.
            if let Some(&last) = out.last() {
                let (l, c) = (last as usize, id as usize);
                if matches!(self.nodes[l].kind, NodeKind::Text)
                    && matches!(self.nodes[c].kind, NodeKind::Text)
                    && self.nodes[l].e == self.nodes[c].s
                {
                    self.nodes[l].e = self.nodes[c].e;
                    continue;
                }
            }
            out.push(id);
        }
        out
    }
}

fn is_email(s: &str) -> bool {
    let Some((user, host)) = s.split_once('@') else { return false };
    !user.is_empty()
        && host.contains('.')
        && user.bytes().all(|c| c.is_ascii_alphanumeric() || b".!#$%&'*+/=?^_`{|}~-".contains(&c))
        && host.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')
}
