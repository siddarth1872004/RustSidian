//! Block-level parser. Works on a table of lines so that containers (quotes, list items) can
//! recurse on the same source text with their prefixes stripped.

use crate::{inline, Align, Fold, NodeId, NodeKind, Options, Span, Tree, NO_NODE};

/// One line of a container. `s..e` is the content with the container prefix removed and without
/// the line terminator; `ne` is the offset just past the terminator.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Line {
    pub s: usize,
    pub e: usize,
    pub ne: usize,
}

pub(crate) fn split_lines(text: &str) -> Vec<Line> {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len() / 40 + 1);
    let mut s = 0;
    while s < b.len() {
        let nl = b[s..].iter().position(|&c| c == b'\n').map_or(b.len(), |p| s + p);
        let ne = (nl + 1).min(b.len());
        let e = if nl > s && b[nl - 1] == b'\r' { nl - 1 } else { nl };
        out.push(Line { s, e, ne });
        s = ne;
    }
    out
}

const MAX_DEPTH: u32 = 48;

pub(crate) struct Parser<'a> {
    pub text: &'a str,
    pub b: &'a [u8],
    /// Offset of the top-level block within `text`; spans are stored relative to it.
    pub base: usize,
    pub opts: Options,
    pub tree: Tree,
    depth: u32,
}

/// Parse the top-level block starting at `lines[i]`. Returns the tree, the index of the first
/// line after the block (trailing blank lines included), and whether the result could change if
/// more lines were appended after `lines`.
pub(crate) fn parse_top(
    text: &str,
    lines: &[Line],
    i: usize,
    opts: Options,
    at_doc_start: bool,
) -> (Tree, usize) {
    let mut p = Parser { text, b: text.as_bytes(), base: lines[i].s, opts, tree: Tree::default(), depth: 0 };
    let mut next;
    if p.is_blank(&lines[i]) {
        next = i;
        while next < lines.len() && p.is_blank(&lines[next]) {
            next += 1;
        }
        p.tree.push(NO_NODE, NodeKind::Blank, Span::new(0, lines[next - 1].ne - p.base));
    } else {
        next = if at_doc_start && opts.front_matter { p.front_matter(lines, i) } else { None }
            .unwrap_or_else(|| p.block_at(lines, i, NO_NODE));
        while next < lines.len() && p.is_blank(&lines[next]) {
            next += 1;
        }
    }
    p.tree.finish();
    (p.tree, next)
}

pub(crate) struct Marker {
    pub ordered: bool,
    pub start: u32,
    /// Bullet character, or the delimiter (`.` / `)`) of an ordered marker.
    pub ch: u8,
    /// Bytes from the start of the marker to the start of the item content.
    pub content_off: usize,
    pub marker_len: usize,
    pub empty: bool,
}

pub(crate) fn indent_of(b: &[u8]) -> (usize, usize) {
    let (mut cols, mut off) = (0, 0);
    for &c in b {
        match c {
            b' ' => cols += 1,
            b'\t' => cols += 4 - cols % 4,
            _ => break,
        }
        off += 1;
    }
    (cols, off)
}

fn trim_end(b: &[u8]) -> &[u8] {
    let mut e = b.len();
    while e > 0 && matches!(b[e - 1], b' ' | b'\t') {
        e -= 1;
    }
    &b[..e]
}

pub(crate) fn fence_open(b: &[u8]) -> Option<(u8, usize)> {
    let ch = *b.first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let n = b.iter().take_while(|&&c| c == ch).count();
    if n < 3 || (ch == b'`' && b[n..].contains(&b'`')) {
        return None;
    }
    Some((ch, n))
}

fn atx(b: &[u8]) -> Option<(u8, usize, usize)> {
    let n = b.iter().take_while(|&&c| c == b'#').count();
    if n == 0 || n > 6 {
        return None;
    }
    if n < b.len() && !matches!(b[n], b' ' | b'\t') {
        return None;
    }
    let (_, ws) = indent_of(&b[n..]);
    let start = n + ws;
    let mut end = trim_end(b).len().max(start);
    // Closing sequence: trailing #s preceded by a space.
    let mut k = end;
    while k > start && b[k - 1] == b'#' {
        k -= 1;
    }
    if k < end && (k == start || matches!(b[k - 1], b' ' | b'\t')) {
        end = trim_end(&b[..k]).len().max(start);
    }
    Some((n as u8, start, end))
}

fn is_hr(b: &[u8]) -> bool {
    let Some(&ch) = b.first() else { return false };
    if !matches!(ch, b'-' | b'*' | b'_') {
        return false;
    }
    let mut n = 0;
    for &c in b {
        if c == ch {
            n += 1;
        } else if c != b' ' && c != b'\t' {
            return false;
        }
    }
    n >= 3
}

fn setext_level(b: &[u8]) -> Option<u8> {
    let t = trim_end(b);
    let ch = *t.first()?;
    if (ch == b'=' || ch == b'-') && t.iter().all(|&c| c == ch) {
        Some(if ch == b'=' { 1 } else { 2 })
    } else {
        None
    }
}

pub(crate) fn list_marker(b: &[u8]) -> Option<Marker> {
    let ch = *b.first()?;
    let (ordered, start, mlen, ch) = if matches!(ch, b'-' | b'*' | b'+') {
        (false, 0, 1, ch)
    } else if ch.is_ascii_digit() {
        let n = b.iter().take_while(|c| c.is_ascii_digit()).count();
        if n > 9 || n >= b.len() || !matches!(b[n], b'.' | b')') {
            return None;
        }
        let start = std::str::from_utf8(&b[..n]).ok()?.parse().ok()?;
        (true, start, n + 1, b[n])
    } else {
        return None;
    };
    let rest = &b[mlen..];
    if rest.is_empty() || trim_end(rest).is_empty() {
        return Some(Marker { ordered, start, ch, content_off: mlen + 1, marker_len: mlen, empty: true });
    }
    if !matches!(rest[0], b' ' | b'\t') {
        return None;
    }
    let (cols, off) = indent_of(rest);
    let content_off = if cols > 4 { mlen + 1 } else { mlen + off };
    Some(Marker { ordered, start, ch, content_off, marker_len: mlen, empty: false })
}

/// Splits a table row into cell ranges (relative to `b`), honoring `\|`.
pub(crate) fn split_cells(b: &[u8]) -> Vec<(usize, usize)> {
    let t = trim_end(b);
    let (_, lead) = indent_of(t);
    let mut cells = Vec::new();
    let mut i = lead;
    if i < t.len() && t[i] == b'|' {
        i += 1;
    }
    let mut start = i;
    while i < t.len() {
        match t[i] {
            b'\\' if i + 1 < t.len() => i += 1,
            b'|' => {
                cells.push((start, i));
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < t.len() {
        cells.push((start, t.len()));
    }
    cells
}

fn delim_row(b: &[u8]) -> Option<Vec<Align>> {
    if !b.contains(&b'-') {
        return None;
    }
    let cells = split_cells(b);
    if cells.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(cells.len());
    for (s, e) in cells {
        let c = trim_end(&b[s..e]);
        let (_, ws) = indent_of(c);
        let c = &c[ws..];
        let left = c.first() == Some(&b':');
        let right = c.len() > 1 && c.last() == Some(&b':');
        let dashes = &c[left as usize..c.len() - right as usize];
        if dashes.is_empty() || !dashes.iter().all(|&x| x == b'-') {
            return None;
        }
        out.push(match (left, right) {
            (true, true) => Align::Center,
            (true, false) => Align::Left,
            (false, true) => Align::Right,
            _ => Align::None,
        });
    }
    Some(out)
}

const BLOCK_TAGS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "body",
    "center",
    "dd",
    "details",
    "dialog",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "iframe",
    "li",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "script",
    "section",
    "style",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "ul",
    "video",
    "audio",
    "img",
];

fn html_block_start(b: &[u8]) -> bool {
    if b.len() < 2 || b[0] != b'<' {
        return false;
    }
    if b.starts_with(b"<!--") || b[1] == b'?' || (b[1] == b'!' && b.len() > 2 && b[2].is_ascii_alphabetic()) {
        return true;
    }
    let s = if b[1] == b'/' { 2 } else { 1 };
    let n = b[s..].iter().take_while(|c| c.is_ascii_alphanumeric()).count();
    if n == 0 {
        return false;
    }
    let after = b.get(s + n).copied();
    if !matches!(after, None | Some(b' ' | b'\t' | b'>' | b'/')) {
        return false;
    }
    let name = &b[s..s + n];
    BLOCK_TAGS.iter().any(|t| t.as_bytes().eq_ignore_ascii_case(name))
}

/// `[^label]:` -> (label range, content offset)
fn footnote_def(b: &[u8]) -> Option<((usize, usize), usize)> {
    if !b.starts_with(b"[^") {
        return None;
    }
    let close = b.iter().position(|&c| c == b']')?;
    if close <= 2 || b.get(close + 1) != Some(&b':') || b[2..close].iter().any(|c| c.is_ascii_whitespace()) {
        return None;
    }
    let (_, ws) = indent_of(&b[close + 2..]);
    Some(((2, close), close + 2 + ws))
}

/// `[label]: dest "title"` -> (label, dest, title) ranges
#[allow(clippy::type_complexity)]
fn linkref_def(b: &[u8]) -> Option<((usize, usize), (usize, usize), (usize, usize))> {
    if b.first() != Some(&b'[') || b.get(1) == Some(&b'^') {
        return None;
    }
    let close = b.iter().position(|&c| c == b']')?;
    if close <= 1 || b.get(close + 1) != Some(&b':') {
        return None;
    }
    let (_, ws) = indent_of(&b[close + 2..]);
    let ds = close + 2 + ws;
    let t = trim_end(b);
    if ds >= t.len() {
        return None;
    }
    let dlen = t[ds..].iter().take_while(|c| !c.is_ascii_whitespace()).count();
    let de = ds + dlen;
    let (_, ws2) = indent_of(&t[de..]);
    let ts = de + ws2;
    let title = if ts < t.len() {
        let q = t[ts];
        if t.len() - ts >= 2 && matches!(q, b'"' | b'\'') && t[t.len() - 1] == q {
            (ts + 1, t.len() - 1)
        } else {
            return None;
        }
    } else {
        (0, 0)
    };
    let (ds, de) =
        if t[ds] == b'<' && t[de - 1] == b'>' && de - ds >= 2 { (ds + 1, de - 1) } else { (ds, de) };
    Some(((1, close), (ds, de), title))
}

impl<'a> Parser<'a> {
    fn line(&self, l: &Line) -> &'a [u8] {
        &self.b[l.s..l.e]
    }
    pub(crate) fn is_blank(&self, l: &Line) -> bool {
        self.line(l).iter().all(|c| matches!(c, b' ' | b'\t'))
    }
    fn sp(&self, s: usize, e: usize) -> Span {
        Span::new(s - self.base, e.max(s) - self.base)
    }

    /// Inside a container the content span of a code or math block includes the container
    /// prefixes of its lines, so the exact lines are recorded as `Text` children.
    fn nested_lines(&mut self, id: NodeId, lines: &[Line]) {
        if self.depth == 0 {
            return;
        }
        for l in lines {
            self.tree.push(id, NodeKind::Text, self.sp(l.s, l.ne));
        }
    }

    fn front_matter(&mut self, lines: &[Line], i: usize) -> Option<usize> {
        if trim_end(self.line(&lines[i])) != b"---" {
            return None;
        }
        let close = (i + 1..lines.len()).find(|&j| {
            let t = trim_end(self.line(&lines[j]));
            t == b"---" || t == b"..."
        })?;
        let content = self.sp(lines[i].ne, lines[close].s);
        self.tree.push(NO_NODE, NodeKind::FrontMatter { content }, self.sp(lines[i].s, lines[close].e));
        Some(close + 1)
    }

    pub(crate) fn blocks(&mut self, lines: &[Line], parent: NodeId) {
        let mut i = 0;
        while i < lines.len() {
            if self.is_blank(&lines[i]) {
                i += 1;
            } else {
                i = self.block_at(lines, i, parent);
            }
        }
    }

    /// True if this line would end a paragraph that precedes it.
    fn interrupts(&self, l: &Line) -> bool {
        let raw = self.line(l);
        let (cols, off) = indent_of(raw);
        if cols >= 4 {
            return false;
        }
        let b = &raw[off..];
        if atx(b).is_some() || fence_open(b).is_some() || is_hr(b) || b.first() == Some(&b'>') {
            return true;
        }
        if let Some(m) = list_marker(b) {
            if !m.empty && (!m.ordered || m.start == 1) {
                return true;
            }
        }
        (self.opts.math && b.starts_with(b"$$"))
            || html_block_start(b)
            || (self.opts.footnotes && footnote_def(b).is_some())
    }

    fn block_at(&mut self, lines: &[Line], i: usize, parent: NodeId) -> usize {
        let l = lines[i];
        let raw = self.line(&l);
        let (cols, off) = indent_of(raw);
        let b = &raw[off..];
        let cs = l.s + off;

        if self.depth >= MAX_DEPTH {
            return self.paragraph(lines, i, parent, true);
        }

        if cols >= 4 {
            let mut j = i + 1;
            let mut last = i;
            while j < lines.len() {
                if self.is_blank(&lines[j]) {
                    j += 1;
                } else if indent_of(self.line(&lines[j])).0 >= 4 {
                    last = j;
                    j += 1;
                } else {
                    break;
                }
            }
            let content = self.sp(l.s, lines[last].ne);
            let kind = NodeKind::CodeBlock { fence: 0, info: Span::EMPTY, content };
            self.tree.push(parent, kind, self.sp(l.s, lines[last].e));
            return last + 1;
        }

        if let Some((ch, n)) = fence_open(b) {
            let info_raw = trim_end(&b[n..]);
            let (_, iws) = indent_of(info_raw);
            let info = self.sp(cs + n + iws, cs + n + info_raw.len());
            let close = (i + 1..lines.len()).find(|&j| {
                let r = self.line(&lines[j]);
                let (c, o) = indent_of(r);
                let t = trim_end(&r[o..]);
                c < 4 && t.len() >= n && t.iter().all(|&x| x == ch)
            });
            let (content_end, end_line) = match close {
                Some(j) => (lines[j].s, j),
                None => (lines[lines.len() - 1].ne, lines.len() - 1),
            };
            let content_start = l.ne.min(content_end);
            let content = self.sp(content_start, content_end);
            let kind = NodeKind::CodeBlock { fence: n.min(255) as u8, info, content };
            let id = self.tree.push(parent, kind, self.sp(l.s, lines[end_line].e));
            let body_end = if close.is_some() { end_line } else { end_line + 1 };
            self.nested_lines(id, &lines[(i + 1).min(body_end)..body_end]);
            return end_line + 1;
        }

        if self.opts.math && b.starts_with(b"$$") {
            let t = trim_end(b);
            if t.len() >= 4 && t.ends_with(b"$$") {
                let content = self.sp(cs + 2, cs + t.len() - 2);
                self.tree.push(parent, NodeKind::MathBlock { content }, self.sp(l.s, l.e));
                return i + 1;
            }
            // An unterminated block runs to the end of its container, like a code fence. That
            // keeps the decision local, which incremental re-parsing relies on.
            let close = (i + 1..lines.len()).find(|&j| trim_end(self.line(&lines[j])).ends_with(b"$$"));
            let (ce, end_line) = match close {
                Some(j) => (lines[j].s + trim_end(self.line(&lines[j])).len() - 2, j),
                None => (lines[lines.len() - 1].e, lines.len() - 1),
            };
            let content = self.sp((cs + 2).min(ce), ce);
            let id = self.tree.push(parent, NodeKind::MathBlock { content }, self.sp(l.s, lines[end_line].e));
            let body_end = if close.is_some() { end_line } else { end_line + 1 };
            self.nested_lines(id, &lines[(i + 1).min(body_end)..body_end]);
            return end_line + 1;
        }

        if self.opts.comments && b.starts_with(b"%%") && !contains(&b[2..], b"%%") {
            let end_line = (i + 1..lines.len())
                .find(|&j| contains(self.line(&lines[j]), b"%%"))
                .unwrap_or(lines.len() - 1);
            self.tree.push(parent, NodeKind::CommentBlock, self.sp(l.s, lines[end_line].e));
            return end_line + 1;
        }

        if let Some((level, s, e)) = atx(b) {
            let id = self.tree.push(parent, NodeKind::Heading { level, setext: false }, self.sp(l.s, l.e));
            inline::parse(self, id, &[(cs + s, cs + e)]);
            return i + 1;
        }

        if is_hr(b) {
            self.tree.push(parent, NodeKind::ThematicBreak, self.sp(l.s, l.e));
            return i + 1;
        }

        if b.first() == Some(&b'>') {
            return self.quote(lines, i, parent);
        }

        if let Some(m) = list_marker(b) {
            return self.list(lines, i, parent, m);
        }

        if self.opts.tables && b.contains(&b'|') && i + 1 < lines.len() {
            if let Some(aligns) = delim_row(self.line(&lines[i + 1])) {
                if split_cells(raw).len() == aligns.len() {
                    return self.table(lines, i, parent, &aligns);
                }
            }
        }

        if html_block_start(b) {
            let mut j = i + 1;
            while j < lines.len() && !self.is_blank(&lines[j]) {
                j += 1;
            }
            self.tree.push(parent, NodeKind::HtmlBlock, self.sp(l.s, lines[j - 1].e));
            return j;
        }

        if self.opts.footnotes {
            if let Some(((ls, le), coff)) = footnote_def(b) {
                let mut sub = vec![Line { s: (cs + coff).min(l.e), e: l.e, ne: l.ne }];
                let mut j = i + 1;
                let mut last = i;
                while j < lines.len() {
                    let r = self.line(&lines[j]);
                    let (c, o) = indent_of(r);
                    if self.is_blank(&lines[j]) {
                        sub.push(lines[j]);
                    } else if c >= 2 {
                        sub.push(Line { s: lines[j].s + o, ..lines[j] });
                        last = j;
                    } else {
                        break;
                    }
                    j += 1;
                }
                sub.truncate(last - i + 1);
                let label = self.sp(cs + ls, cs + le);
                let id = self.tree.push(parent, NodeKind::FootnoteDef { label }, self.sp(l.s, lines[last].e));
                self.depth += 1;
                self.blocks(&sub, id);
                self.depth -= 1;
                return last + 1;
            }
        }

        if let Some((label, dest, title)) = linkref_def(b) {
            let kind = NodeKind::LinkRefDef {
                label: self.sp(cs + label.0, cs + label.1),
                dest: self.sp(cs + dest.0, cs + dest.1),
                title: self.sp(cs + title.0, cs + title.1),
            };
            self.tree.push(parent, kind, self.sp(l.s, l.e));
            return i + 1;
        }

        self.paragraph(lines, i, parent, false)
    }

    fn paragraph(&mut self, lines: &[Line], i: usize, parent: NodeId, greedy: bool) -> usize {
        let mut j = i + 1;
        let mut setext = None;
        while j < lines.len() && !self.is_blank(&lines[j]) {
            if !greedy {
                let r = self.line(&lines[j]);
                let (c, o) = indent_of(r);
                if c < 4 {
                    if let Some(level) = setext_level(&r[o..]) {
                        setext = Some(level);
                        break;
                    }
                }
                if self.interrupts(&lines[j]) {
                    break;
                }
            }
            j += 1;
        }
        let mut segs = Vec::with_capacity(j - i);
        for l in &lines[i..j] {
            let (_, o) = indent_of(self.line(l));
            segs.push((l.s + o, l.e));
        }
        if let Some(last) = segs.last_mut() {
            last.1 = last.0 + trim_end(&self.b[last.0..last.1]).len();
        }
        let (kind, end) = match setext {
            Some(level) => (NodeKind::Heading { level, setext: true }, j),
            None => (NodeKind::Paragraph, j - 1),
        };
        let id = self.tree.push(parent, kind, self.sp(lines[i].s, lines[end].e));
        inline::parse(self, id, &segs);
        end + 1
    }

    fn quote(&mut self, lines: &[Line], i: usize, parent: NodeId) -> usize {
        let mut sub = Vec::new();
        let mut j = i;
        let mut prev_text = false;
        while j < lines.len() {
            let l = lines[j];
            let raw = self.line(&l);
            let (cols, off) = indent_of(raw);
            if cols < 4 && raw.get(off) == Some(&b'>') {
                let mut s = l.s + off + 1;
                if self.b.get(s) == Some(&b' ') && s < l.e {
                    s += 1;
                }
                let nl = Line { s, ..l };
                prev_text = !self.is_blank(&nl);
                sub.push(nl);
            } else if prev_text && !self.is_blank(&l) && !self.interrupts(&l) {
                // Lazy continuation of a quoted paragraph.
                sub.push(l);
            } else {
                break;
            }
            j += 1;
        }
        let span = self.sp(lines[i].s, lines[j - 1].e);
        let mut kind = NodeKind::BlockQuote;
        let mut body = &sub[..];
        let mut title_seg = None;
        if self.opts.callouts {
            let first = self.line(&sub[0]);
            if first.starts_with(b"[!") {
                if let Some(close) = first.iter().position(|&c| c == b']') {
                    let fs = sub[0].s;
                    let mut k = close + 1;
                    let fold = match first.get(k) {
                        Some(b'+') => Fold::Open,
                        Some(b'-') => Fold::Closed,
                        _ => Fold::None,
                    };
                    if fold != Fold::None {
                        k += 1;
                    }
                    let (_, ws) = indent_of(&first[k..]);
                    let ts = fs + k + ws;
                    let te = fs + trim_end(first).len();
                    if close > 2 {
                        let title = self.sp(ts.min(te), te);
                        kind = NodeKind::Callout { kind: self.sp(fs + 2, fs + close), fold, title };
                        if !title.is_empty() {
                            title_seg = Some((ts, te));
                        }
                        body = &sub[1..];
                    }
                }
            }
        }
        let id = self.tree.push(parent, kind, span);
        if let Some(seg) = title_seg {
            // The title is the callout's first child, a Heading of level 0.
            let h = self.tree.push(id, NodeKind::Heading { level: 0, setext: false }, self.sp(seg.0, seg.1));
            inline::parse(self, h, &[seg]);
        }
        self.depth += 1;
        self.blocks(body, id);
        self.depth -= 1;
        j
    }

    fn list(&mut self, lines: &[Line], i: usize, parent: NodeId, first: Marker) -> usize {
        let kind = NodeKind::List { ordered: first.ordered, start: first.start, tight: true };
        let list_id = self.tree.push(parent, kind, Span::EMPTY);
        let mut j = i;
        let mut tight = true;
        let mut last_content = i;
        let mut marker = Some(first);
        while let Some(m) = marker.take() {
            let l = lines[j];
            let (base_cols, off) = indent_of(self.line(&l));
            let ms = l.s + off;
            let width = base_cols + m.content_off;
            let mut sub = vec![Line { s: (ms + m.content_off).min(l.e), ..l }];
            let item_start = j;
            let mut item_last = j;
            let mut k = j + 1;
            let mut blank_run = 0;
            let mut blank_inside = false;
            while k < lines.len() {
                let lk = lines[k];
                let raw = self.line(&lk);
                if self.is_blank(&lk) {
                    blank_run += 1;
                    sub.push(Line { s: lk.e, ..lk });
                    k += 1;
                    continue;
                }
                let (c, o) = indent_of(raw);
                if c >= width {
                    // Strip `width` columns of indentation.
                    let mut cols = 0;
                    let mut so = 0;
                    while cols < width && so < o {
                        cols += if raw[so] == b'\t' { 4 - cols % 4 } else { 1 };
                        so += 1;
                    }
                    sub.push(Line { s: lk.s + so, ..lk });
                    if blank_run > 0 {
                        blank_inside = true;
                    }
                } else if blank_run == 0 && list_marker(&raw[o..]).is_none() && !self.interrupts(&lk) {
                    sub.push(Line { s: lk.s + o, ..lk });
                } else {
                    break;
                }
                blank_run = 0;
                item_last = k;
                k += 1;
            }
            sub.truncate(item_last - item_start + 1);
            // Task marker.
            let mut task = None;
            if self.opts.task_lists && !m.empty {
                let c = self.line(&sub[0]);
                if c.len() >= 3
                    && c[0] == b'['
                    && c[2] == b']'
                    && (c.len() == 3 || c[3] == b' ')
                    && c[1] != b'\n'
                {
                    task = Some(c[1]);
                    sub[0].s += (4).min(c.len());
                }
            }
            let ik = NodeKind::ListItem { task, marker: self.sp(ms, ms + m.marker_len) };
            let item_id = self.tree.push(list_id, ik, self.sp(l.s, lines[item_last].e));
            self.depth += 1;
            self.blocks(&sub, item_id);
            self.depth -= 1;
            if blank_inside {
                tight = false;
            }
            last_content = item_last;
            // Is the next non-blank line a sibling item?
            j = k;
            if j < lines.len() && !self.is_blank(&lines[j]) {
                let raw = self.line(&lines[j]);
                let (c, o) = indent_of(raw);
                if c < 4 && !is_hr(&raw[o..]) {
                    if let Some(nm) = list_marker(&raw[o..]) {
                        if nm.ordered == m.ordered && nm.ch == m.ch {
                            if blank_run > 0 {
                                tight = false;
                            }
                            marker = Some(nm);
                        }
                    }
                }
            }
        }
        let span = self.sp(lines[i].s, lines[last_content].e);
        let n = &mut self.tree.nodes[list_id as usize];
        n.span = span;
        if let NodeKind::List { tight: t, .. } = &mut n.kind {
            *t = tight;
        }
        last_content + 1
    }

    fn table(&mut self, lines: &[Line], i: usize, parent: NodeId, aligns: &[Align]) -> usize {
        let mut j = i + 2;
        while j < lines.len() && !self.is_blank(&lines[j]) && self.line(&lines[j]).contains(&b'|') {
            j += 1;
        }
        let kind = NodeKind::Table { cols: aligns.len().min(u16::MAX as usize) as u16 };
        let tid = self.tree.push(parent, kind, self.sp(lines[i].s, lines[j - 1].e));
        for (r, l) in lines[i..j].iter().enumerate() {
            if r == 1 {
                continue;
            }
            let raw = self.line(l);
            let rid = self.tree.push(tid, NodeKind::TableRow { header: r == 0 }, self.sp(l.s, l.e));
            for (c, (s, e)) in split_cells(raw).into_iter().enumerate().take(aligns.len()) {
                let cell = &raw[s..e];
                let (_, ws) = indent_of(cell);
                let (cs, ce) =
                    (l.s + s + ws.min(cell.len()), l.s + s + trim_end(cell).len().max(ws.min(cell.len())));
                let cid =
                    self.tree.push(rid, NodeKind::TableCell { align: aligns[c] }, self.sp(l.s + s, l.s + e));
                if ce > cs {
                    inline::parse(self, cid, &[(cs, ce)]);
                }
            }
        }
        j
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}
