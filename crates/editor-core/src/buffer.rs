//! Text buffer: rope + incrementally maintained parse tree + undo tree.

use md_engine::{Document, Edit, Options, TextSource};
use ropey::Rope;
use std::borrow::Cow;

/// Replace `start..end` (byte offsets on char boundaries) with `text`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Change {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// A selection range. `anchor == head` is a cursor.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Sel {
    pub anchor: usize,
    pub head: usize,
}

impl Sel {
    pub fn cursor(at: usize) -> Sel {
        Sel { anchor: at, head: at }
    }
    pub fn new(anchor: usize, head: usize) -> Sel {
        Sel { anchor, head }
    }
    pub fn start(self) -> usize {
        self.anchor.min(self.head)
    }
    pub fn end(self) -> usize {
        self.anchor.max(self.head)
    }
    pub fn is_empty(self) -> bool {
        self.anchor == self.head
    }
}

struct RopeText<'a>(&'a Rope);

impl TextSource for RopeText<'_> {
    fn len(&self) -> usize {
        self.0.len_bytes()
    }
    fn slice_from(&self, start: usize, min_len: usize) -> Cow<'_, str> {
        let total = self.0.len_bytes();
        let mut end = start.saturating_add(min_len).min(total);
        while end < total && self.0.try_byte_to_char(end).is_ok_and(|c| self.0.char_to_byte(c) != end) {
            end += 1;
        }
        Cow::from(self.0.byte_slice(start..end))
    }
}

struct Revision {
    parent: usize,
    /// Changes that turn the parent's text into this revision's text.
    forward: Vec<Change>,
    /// Changes that turn this revision's text back into the parent's.
    inverse: Vec<Change>,
    sels_before: Vec<Sel>,
    sels_after: Vec<Sel>,
    /// Child to follow on redo (the most recently created or visited).
    redo_child: Option<usize>,
    time_ms: u64,
    /// Whether the next typing edit may be merged into this revision.
    mergeable: bool,
}

/// How an edit relates to undo grouping.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditKind {
    /// Consecutive typing within the merge window becomes one undo step.
    Typing,
    /// Always its own undo step.
    Other,
}

pub struct Buffer {
    rope: Rope,
    doc: Document,
    pub sels: Vec<Sel>,
    revisions: Vec<Revision>,
    current: usize,
    /// Revision that matches the file on disk.
    saved: usize,
    /// Bumped on every text change.
    pub version: u64,
    /// Typing edits closer together than this are merged into one undo step.
    pub merge_window_ms: u64,
    /// Change sets applied since `take_changes` was last called, oldest first.
    log: Vec<Vec<Change>>,
    log_overflowed: bool,
}

/// How many change sets are kept for `take_changes` before the log gives up.
const CHANGE_LOG_MAX: usize = 256;

fn char_boundary(rope: &Rope, at: usize) -> usize {
    let at = at.min(rope.len_bytes());
    rope.char_to_byte(rope.byte_to_char(at))
}

impl Buffer {
    pub fn new(text: &str) -> Buffer {
        Buffer::with_options(text, Options::default())
    }

    pub fn with_options(text: &str, opts: Options) -> Buffer {
        let root = Revision {
            parent: 0,
            forward: Vec::new(),
            inverse: Vec::new(),
            sels_before: Vec::new(),
            sels_after: Vec::new(),
            redo_child: None,
            time_ms: 0,
            mergeable: false,
        };
        Buffer {
            rope: Rope::from_str(text),
            doc: Document::parse(text, opts),
            sels: vec![Sel::cursor(0)],
            revisions: vec![root],
            current: 0,
            saved: 0,
            version: 0,
            merge_window_ms: 700,
            log: Vec::new(),
            log_overflowed: false,
        }
    }

    pub fn rope(&self) -> &Rope {
        &self.rope
    }
    pub fn doc(&self) -> &Document {
        &self.doc
    }
    pub fn len(&self) -> usize {
        self.rope.len_bytes()
    }
    pub fn is_empty(&self) -> bool {
        self.rope.len_bytes() == 0
    }
    pub fn text(&self) -> String {
        self.rope.to_string()
    }
    pub fn slice(&self, start: usize, end: usize) -> Cow<'_, str> {
        let (s, e) = (char_boundary(&self.rope, start), char_boundary(&self.rope, end));
        Cow::from(self.rope.byte_slice(s.min(e)..e))
    }
    pub fn is_modified(&self) -> bool {
        self.current != self.saved
    }
    pub fn mark_saved(&mut self) {
        self.saved = self.current;
        self.revisions[self.current].mergeable = false;
    }

    // ---- lines ----
    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }
    pub fn line_of(&self, at: usize) -> usize {
        self.rope.byte_to_line(at.min(self.len()))
    }
    pub fn line_start(&self, line: usize) -> usize {
        self.rope.line_to_byte(line.min(self.rope.len_lines()))
    }
    /// End of the line's content, before its terminator.
    pub fn line_end(&self, line: usize) -> usize {
        if line + 1 >= self.rope.len_lines() {
            return self.len();
        }
        let mut e = self.rope.line_to_byte(line + 1);
        while e > self.line_start(line) && matches!(self.rope.byte(e - 1), b'\n' | b'\r') {
            e -= 1;
        }
        e
    }
    pub fn line_text(&self, line: usize) -> Cow<'_, str> {
        self.slice(self.line_start(line), self.line_end(line))
    }

    /// Apply raw changes to the rope and the parse tree. Changes must be sorted and disjoint.
    /// Returns the inverse changes (in the coordinates of the resulting text).
    fn apply_raw(&mut self, changes: &[Change]) -> Vec<Change> {
        let mut inverse = Vec::with_capacity(changes.len());
        let mut delta: isize = 0;
        for c in changes {
            let start = (c.start as isize + delta) as usize;
            let end = (c.end as isize + delta) as usize;
            let (cs, ce) = (self.rope.byte_to_char(start), self.rope.byte_to_char(end));
            let removed: String = self.rope.slice(cs..ce).into();
            self.rope.remove(cs..ce);
            self.rope.insert(cs, &c.text);
            self.doc.edit(&RopeText(&self.rope), Edit { start, old_end: end, new_end: start + c.text.len() });
            inverse.push(Change { start, end: start + c.text.len(), text: removed });
            delta += c.text.len() as isize - (c.end - c.start) as isize;
        }
        self.version += 1;
        if self.log.len() < CHANGE_LOG_MAX {
            self.log.push(changes.to_vec());
        } else {
            self.log_overflowed = true;
        }
        inverse
    }

    /// The change sets applied (by edits, undo and redo) since the last call, for keeping
    /// positions held outside the buffer valid with `map_offset`. `None` means too many
    /// accumulated to track and such positions must be discarded.
    pub fn take_changes(&mut self) -> Option<Vec<Vec<Change>>> {
        let log = std::mem::take(&mut self.log);
        (!std::mem::take(&mut self.log_overflowed)).then_some(log)
    }

    fn normalize(&self, changes: &mut Vec<Change>) {
        for c in changes.iter_mut() {
            c.start = char_boundary(&self.rope, c.start);
            c.end = char_boundary(&self.rope, c.end).max(c.start);
        }
        changes.sort_by_key(|c| (c.start, c.end));
        // Drop overlapping changes (keeps the first); callers produce disjoint changes.
        let mut last_end = 0;
        let mut first = true;
        changes.retain(|c| {
            let ok = first || c.start >= last_end;
            if ok {
                last_end = c.end;
                first = false;
            }
            ok
        });
        changes.retain(|c| c.start != c.end || !c.text.is_empty());
    }

    /// Apply a transaction: a set of changes plus the selections afterwards.
    /// `sels_after` of `None` maps the current selections through the changes.
    pub fn apply(
        &mut self,
        mut changes: Vec<Change>,
        sels_after: Option<Vec<Sel>>,
        kind: EditKind,
        now_ms: u64,
    ) {
        self.normalize(&mut changes);
        if changes.is_empty() {
            if let Some(s) = sels_after {
                self.set_sels(s);
            }
            return;
        }
        let sels_before = self.sels.clone();
        let mapped =
            sels_after.unwrap_or_else(|| sels_before.iter().map(|s| map_sel(*s, &changes)).collect());
        let inverse = self.apply_raw(&changes);
        self.set_sels(mapped);
        let cur = &mut self.revisions[self.current];
        let merge = kind == EditKind::Typing
            && cur.mergeable
            && self.current != self.saved
            && self.current != 0
            && now_ms.saturating_sub(cur.time_ms) <= self.merge_window_ms
            && cur.redo_child.is_none();
        if merge {
            // Keep both steps inside one revision: undo replays the inverses newest first.
            cur.forward.push(Change { start: usize::MAX, end: 0, text: String::new() });
            cur.forward.extend(changes);
            let mut inv = inverse;
            inv.push(Change { start: usize::MAX, end: 0, text: String::new() });
            inv.append(&mut cur.inverse);
            cur.inverse = inv;
            cur.sels_after = self.sels.clone();
            cur.time_ms = now_ms;
            return;
        }
        self.revisions.push(Revision {
            parent: self.current,
            forward: changes,
            inverse,
            sels_before,
            sels_after: self.sels.clone(),
            redo_child: None,
            time_ms: now_ms,
            mergeable: kind == EditKind::Typing,
        });
        let id = self.revisions.len() - 1;
        self.revisions[self.current].redo_child = Some(id);
        self.current = id;
    }

    /// Replay a change list that may contain several steps separated by sentinel entries.
    fn replay(&mut self, steps: &[Change]) {
        for step in steps.split(|c| c.start == usize::MAX) {
            if !step.is_empty() {
                self.apply_raw(step);
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        self.current != 0
    }
    pub fn can_redo(&self) -> bool {
        self.revisions[self.current].redo_child.is_some()
    }

    pub fn undo(&mut self) -> bool {
        if self.current == 0 {
            return false;
        }
        let id = self.current;
        let inverse = self.revisions[id].inverse.clone();
        self.replay(&inverse);
        let sels = self.revisions[id].sels_before.clone();
        self.set_sels(sels);
        let parent = self.revisions[id].parent;
        self.revisions[id].mergeable = false;
        self.revisions[parent].redo_child = Some(id);
        self.current = parent;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(child) = self.revisions[self.current].redo_child else { return false };
        let forward = self.revisions[child].forward.clone();
        self.replay(&forward);
        let sels = self.revisions[child].sels_after.clone();
        self.set_sels(sels);
        self.current = child;
        true
    }

    /// Other branches of the undo tree at the current revision (edits made after an undo).
    pub fn redo_branches(&self) -> Vec<usize> {
        (1..self.revisions.len())
            .filter(|&i| self.revisions[i].parent == self.current && i != self.current)
            .collect()
    }

    /// Choose which branch `redo` follows.
    pub fn select_redo_branch(&mut self, revision: usize) -> bool {
        if revision < self.revisions.len() && revision != 0 && self.revisions[revision].parent == self.current
        {
            self.revisions[self.current].redo_child = Some(revision);
            true
        } else {
            false
        }
    }

    /// Replace selections: clamp, snap to char boundaries, sort and merge overlaps.
    pub fn set_sels(&mut self, mut sels: Vec<Sel>) {
        for s in &mut sels {
            s.anchor = char_boundary(&self.rope, s.anchor);
            s.head = char_boundary(&self.rope, s.head);
        }
        sels.sort_by_key(|s| (s.start(), s.end()));
        let mut out: Vec<Sel> = Vec::with_capacity(sels.len());
        for s in sels {
            match out.last_mut() {
                Some(last)
                    if s.start() < last.end()
                        || (s.start() == last.end() && (s.is_empty() || last.is_empty())) =>
                {
                    let forward = last.head >= last.anchor;
                    let (a, b) = (last.start(), last.end().max(s.end()));
                    *last = if forward { Sel::new(a, b) } else { Sel::new(b, a) };
                }
                _ => out.push(s),
            }
        }
        if out.is_empty() {
            out.push(Sel::cursor(0));
        }
        self.sels = out;
    }
}

/// Map an offset through sorted, disjoint changes. `after` decides which side an insertion at
/// exactly this offset pushes the position to.
pub fn map_offset(at: usize, changes: &[Change], after: bool) -> usize {
    let mut delta: isize = 0;
    for c in changes {
        if c.end < at
            || (c.end == at && (after || c.start < c.end)) && c.start < at
            || (c.start == at && c.end == at && after)
        {
            delta += c.text.len() as isize - (c.end - c.start) as isize;
        } else if c.start < at {
            // Inside a replaced range: move to the end of the replacement.
            return (c.start as isize + delta) as usize + c.text.len();
        } else {
            break;
        }
    }
    (at as isize + delta) as usize
}

fn map_sel(s: Sel, changes: &[Change]) -> Sel {
    if s.is_empty() {
        Sel::cursor(map_offset(s.head, changes, true))
    } else {
        let (a, b) = (map_offset(s.start(), changes, false), map_offset(s.end(), changes, true));
        if s.head >= s.anchor {
            Sel::new(a, b.max(a))
        } else {
            Sel::new(b.max(a), a)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(start: usize, end: usize, text: &str) -> Change {
        Change { start, end, text: text.into() }
    }

    #[test]
    fn apply_keeps_parse_tree_in_sync() {
        let mut b = Buffer::new("# Title\n\ntext\n");
        b.apply(vec![ch(9, 13, "[[Link]] here"), ch(2, 7, "New")], None, EditKind::Other, 0);
        assert_eq!(b.text(), "# New\n\n[[Link]] here\n");
        assert_eq!(b.doc(), &md_engine::parse(&b.text()));
        assert!(b.is_modified());
    }

    #[test]
    fn undo_redo_restores_text_and_selections() {
        let mut b = Buffer::new("hello world");
        b.set_sels(vec![Sel::cursor(5)]);
        b.apply(vec![ch(5, 5, ",")], None, EditKind::Other, 0);
        assert_eq!((b.text().as_str(), b.sels[0]), ("hello, world", Sel::cursor(6)));
        b.apply(vec![ch(0, 5, "HELLO"), ch(7, 12, "WORLD")], None, EditKind::Other, 0);
        assert_eq!(b.text(), "HELLO, WORLD");
        assert!(b.undo());
        assert_eq!(b.text(), "hello, world");
        assert!(b.undo());
        assert_eq!((b.text().as_str(), b.sels[0]), ("hello world", Sel::cursor(5)));
        assert!(!b.undo() && !b.is_modified());
        assert!(b.redo() && b.redo() && !b.redo());
        assert_eq!(b.text(), "HELLO, WORLD");
        assert_eq!(b.doc(), &md_engine::parse("HELLO, WORLD"));
    }

    #[test]
    fn typing_merges_within_window_only() {
        let mut b = Buffer::new("");
        for (i, c) in "abc".chars().enumerate() {
            b.apply(vec![ch(i, i, &c.to_string())], None, EditKind::Typing, 100 * i as u64);
        }
        b.apply(vec![ch(3, 3, "d")], None, EditKind::Typing, 5_000);
        assert_eq!(b.text(), "abcd");
        b.undo();
        assert_eq!(b.text(), "abc", "the late keystroke is its own step");
        b.undo();
        assert_eq!(b.text(), "", "fast typing is one step");
        b.redo();
        assert_eq!((b.text().as_str(), b.sels[0]), ("abc", Sel::cursor(3)));
    }

    #[test]
    fn undo_tree_keeps_abandoned_branches() {
        let mut b = Buffer::new("x");
        b.apply(vec![ch(1, 1, "1")], None, EditKind::Other, 0);
        b.undo();
        b.apply(vec![ch(1, 1, "2")], None, EditKind::Other, 0);
        assert_eq!(b.text(), "x2");
        b.undo();
        let branches = b.redo_branches();
        assert_eq!(branches.len(), 2);
        assert!(b.select_redo_branch(branches[0]));
        b.redo();
        assert_eq!(b.text(), "x1", "the first branch is still reachable");
    }

    #[test]
    fn saved_state_tracking() {
        let mut b = Buffer::new("a");
        b.apply(vec![ch(1, 1, "b")], None, EditKind::Typing, 0);
        b.mark_saved();
        assert!(!b.is_modified());
        b.apply(vec![ch(2, 2, "c")], None, EditKind::Typing, 10);
        assert!(b.is_modified());
        b.undo();
        assert_eq!(b.text(), "ab");
        assert!(!b.is_modified(), "undo back to the saved revision");
    }

    #[test]
    fn selections_merge_and_snap() {
        let mut b = Buffer::new("héllo wörld");
        b.set_sels(vec![Sel::new(0, 4), Sel::new(3, 7), Sel::cursor(2), Sel::cursor(100)]);
        assert_eq!(b.sels, vec![Sel::new(0, 7), Sel::cursor(b.len())]);
        b.set_sels(vec![Sel::cursor(2)]);
        assert_eq!(b.sels, vec![Sel::cursor(1)], "offset inside é snaps to its start");
    }

    #[test]
    fn lines() {
        let b = Buffer::new("one\r\ntwo\n\nlast");
        assert_eq!(b.line_count(), 4);
        assert_eq!((b.line_start(1), b.line_end(1), &*b.line_text(1)), (5, 8, "two"));
        assert_eq!(&*b.line_text(0), "one");
        assert_eq!(&*b.line_text(2), "");
        assert_eq!((b.line_of(9), b.line_end(3)), (2, b.len()));
    }

    #[test]
    fn offsets_map_through_changes() {
        let c = vec![ch(2, 4, "XYZ"), ch(10, 10, "!")];
        assert_eq!(map_offset(1, &c, true), 1);
        assert_eq!(map_offset(3, &c, true), 5, "inside a replaced range");
        assert_eq!(map_offset(6, &c, true), 7);
        assert_eq!(map_offset(10, &c, true), 12, "insertion at a cursor pushes it");
        assert_eq!(map_offset(10, &c, false), 11);
        assert_eq!(map_offset(20, &c, true), 22);
    }
}
