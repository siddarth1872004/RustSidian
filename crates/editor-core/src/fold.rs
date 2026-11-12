//! Fold ranges: heading sections, list items with children, callouts/quotes, code blocks.

use md_engine::{Document, NodeKind};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FoldRange {
    /// Offset of the line that stays visible (the heading or first line).
    pub header: usize,
    /// Hidden range: from the end of the header line to the end of the section.
    pub start: usize,
    pub end: usize,
}

fn first_line_end(text: &str, from: usize) -> usize {
    text[from..].find('\n').map_or(text.len(), |p| from + p)
}

fn trim_end(text: &str, start: usize, mut end: usize) -> usize {
    while end > start && text.as_bytes()[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    end
}

/// All foldable ranges of a document, sorted by header offset.
pub fn fold_ranges(doc: &Document, text: &str) -> Vec<FoldRange> {
    let mut out = Vec::new();
    let mut push = |header: usize, end: usize| {
        let start = first_line_end(text, header);
        let end = trim_end(text, start, end.min(text.len()));
        if end > start {
            out.push(FoldRange { header, start, end });
        }
    };
    // Heading sections: up to the next heading of the same or a higher level.
    let heads: Vec<(usize, u8)> = doc
        .blocks
        .iter()
        .filter_map(|b| match b.kind() {
            NodeKind::Heading { level, .. } => Some((b.start, level)),
            _ => None,
        })
        .collect();
    for (i, &(at, level)) in heads.iter().enumerate() {
        let end = heads[i + 1..].iter().find(|h| h.1 <= level).map_or(text.len(), |h| h.0);
        push(at, end);
    }
    for b in &doc.blocks {
        for (_, n) in b.tree.iter() {
            let (s, e) = (b.start + n.span.start as usize, b.start + n.span.end as usize);
            match n.kind {
                NodeKind::ListItem { .. }
                | NodeKind::Callout { .. }
                | NodeKind::BlockQuote
                | NodeKind::CodeBlock { .. } => push(s, e),
                _ => {}
            }
        }
    }
    out.sort_by_key(|f| (f.header, f.end));
    out.dedup_by_key(|f| f.header);
    out
}

/// The set of folded sections of one editor, kept valid across edits by header offset.
#[derive(Clone, Debug, Default)]
pub struct Folds {
    headers: Vec<usize>,
}

impl Folds {
    pub fn toggle(&mut self, header: usize) {
        match self.headers.binary_search(&header) {
            Ok(i) => {
                self.headers.remove(i);
            }
            Err(i) => self.headers.insert(i, header),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.headers.is_empty()
    }
    pub fn fold(&mut self, header: usize) {
        if let Err(i) = self.headers.binary_search(&header) {
            self.headers.insert(i, header);
        }
    }
    pub fn is_folded(&self, header: usize) -> bool {
        self.headers.binary_search(&header).is_ok()
    }
    pub fn clear(&mut self) {
        self.headers.clear();
    }
    /// Keep fold anchors pointing at the same text after an edit.
    pub fn map(&mut self, changes: &[crate::buffer::Change]) {
        for h in &mut self.headers {
            *h = crate::buffer::map_offset(*h, changes, false);
        }
        self.headers.sort_unstable();
        self.headers.dedup();
    }
    /// Hidden ranges, merged, for the current document. Folds whose header vanished are dropped.
    pub fn hidden(&mut self, ranges: &[FoldRange]) -> Vec<(usize, usize)> {
        self.headers.retain(|h| ranges.binary_search_by_key(h, |r| r.header).is_ok());
        let mut out: Vec<(usize, usize)> = Vec::new();
        for r in ranges.iter().filter(|r| self.is_folded(r.header)) {
            match out.last_mut() {
                Some(last) if r.start <= last.1 => last.1 = last.1.max(r.end),
                _ => out.push((r.start, r.end)),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "# A\ntext\n\n## B\n- item\n  - child\n- solo\n\n# C\n```\ncode\n```\n";

    #[test]
    fn ranges() {
        let doc = md_engine::parse(DOC);
        let r = fold_ranges(&doc, DOC);
        let shown: Vec<(&str, &str)> =
            r.iter().map(|f| (DOC[f.header..f.start].trim(), &DOC[f.start..f.end])).collect();
        assert_eq!(
            shown,
            vec![
                ("# A", "\ntext\n\n## B\n- item\n  - child\n- solo"),
                ("## B", "\n- item\n  - child\n- solo"),
                ("- item", "\n  - child"),
                ("# C", "\n```\ncode\n```"),
                ("```", "\ncode\n```"),
            ]
        );
    }

    #[test]
    fn folding_state() {
        let doc = md_engine::parse(DOC);
        let r = fold_ranges(&doc, DOC);
        let mut f = Folds::default();
        f.toggle(0);
        f.toggle(DOC.find("## B").unwrap());
        assert_eq!(f.hidden(&r), vec![(r[0].start, r[0].end)], "nested folds merge");
        f.toggle(0);
        assert_eq!(f.hidden(&r).len(), 1);
        f.toggle(7);
        assert_eq!(f.hidden(&r).len(), 1, "a fold without a header is dropped");
        f.map(&[crate::buffer::Change { start: 0, end: 0, text: "xx".into() }]);
        assert!(f.is_folded(DOC.find("## B").unwrap() + 2));
    }
}
