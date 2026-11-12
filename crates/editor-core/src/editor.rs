//! Editing commands on top of [`Buffer`]: multi-cursor movement and typing, smart lists,
//! auto-pairing, indentation, Markdown formatting toggles.

use crate::buffer::{Buffer, Change, EditKind, Sel};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Motion {
    Left,
    Right,
    WordLeft,
    WordRight,
    /// Start of the next word (vim's `w`).
    NextWord,
    /// First non-blank character, or column 0 if already there.
    LineStart,
    LineEnd,
    Up,
    Down,
    DocStart,
    DocEnd,
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub auto_pair_brackets: bool,
    pub auto_pair_markdown: bool,
    pub smart_lists: bool,
    pub tab_size: usize,
    pub use_tabs: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            auto_pair_brackets: true,
            auto_pair_markdown: true,
            smart_lists: true,
            tab_size: 4,
            use_tabs: true,
        }
    }
}

pub struct Editor {
    pub buf: Buffer,
    pub settings: Settings,
    /// Remembered column (in chars) for vertical movement, per selection.
    goal: Vec<Option<usize>>,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The list/quote prefix of a line: (indent, marker text including trailing space, is the item empty).
fn list_prefix(line: &str) -> Option<(String, String, bool)> {
    let indent_len = line.len() - line.trim_start_matches([' ', '\t']).len();
    let (indent, rest) = line.split_at(indent_len);
    let mut marker = String::new();
    let mut r = rest;
    // Quote markers may precede a list marker.
    while let Some(after) = r.strip_prefix('>') {
        marker.push('>');
        r = after;
        if let Some(a) = r.strip_prefix(' ') {
            marker.push(' ');
            r = a;
        }
    }
    let bullet =
        r.chars().next().filter(|c| matches!(c, '-' | '*' | '+')).filter(|_| r[1..].starts_with(' '));
    if let Some(b) = bullet {
        marker.push(b);
        marker.push(' ');
        r = &r[2..];
        let rb = r.as_bytes();
        if rb.len() >= 4 && rb[0] == b'[' && rb[2] == b']' && rb[3] == b' ' {
            marker.push_str("[ ] ");
            r = &r[4..];
        }
    } else {
        let digits = r.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0
            && digits < 10
            && r[digits..].starts_with(['.', ')'])
            && r[digits + 1..].starts_with(' ')
        {
            let n: u64 = r[..digits].parse().unwrap_or(0);
            marker.push_str(&format!("{}{} ", n + 1, &r[digits..digits + 1]));
            r = &r[digits + 2..];
        }
    }
    if marker.is_empty() {
        return None;
    }
    Some((indent.to_string(), marker, r.trim().is_empty()))
}

const PAIRS: &[(char, char)] = &[('(', ')'), ('[', ']'), ('{', '}'), ('"', '"'), ('\'', '\'')];
const MD_PAIRS: &[(char, char)] =
    &[('*', '*'), ('_', '_'), ('`', '`'), ('=', '='), ('~', '~'), ('$', '$'), ('%', '%')];

impl Editor {
    pub fn new(text: &str) -> Editor {
        Editor { buf: Buffer::new(text), settings: Settings::default(), goal: Vec::new() }
    }

    fn char_before(&self, at: usize) -> Option<char> {
        self.buf.slice(at.saturating_sub(4), at).chars().next_back()
    }
    fn char_after(&self, at: usize) -> Option<char> {
        self.buf.slice(at, (at + 4).min(self.buf.len())).chars().next()
    }
    fn prev(&self, at: usize) -> usize {
        self.char_before(at).map_or(at, |c| at - c.len_utf8())
    }
    fn next(&self, at: usize) -> usize {
        self.char_after(at).map_or(at, |c| at + c.len_utf8())
    }

    fn word_left(&self, mut at: usize) -> usize {
        while self.char_before(at).is_some_and(|c| !is_word(c)) {
            at = self.prev(at);
        }
        while self.char_before(at).is_some_and(is_word) {
            at = self.prev(at);
        }
        at
    }
    fn word_right(&self, mut at: usize) -> usize {
        while self.char_after(at).is_some_and(|c| !is_word(c)) {
            at = self.next(at);
        }
        while self.char_after(at).is_some_and(is_word) {
            at = self.next(at);
        }
        at
    }

    fn col_of(&self, at: usize) -> usize {
        let line = self.buf.line_of(at);
        self.buf.slice(self.buf.line_start(line), at).chars().count()
    }
    fn at_col(&self, line: usize, col: usize) -> usize {
        let text = self.buf.line_text(line);
        let off = text.char_indices().nth(col).map_or(text.len(), |(i, _)| i);
        self.buf.line_start(line) + off
    }

    fn moved(&self, at: usize, m: Motion, goal: &mut Option<usize>) -> usize {
        let line = self.buf.line_of(at);
        if !matches!(m, Motion::Up | Motion::Down) {
            *goal = None;
        }
        match m {
            Motion::Left => self.prev(at),
            Motion::Right => self.next(at),
            Motion::WordLeft => self.word_left(at),
            Motion::WordRight => self.word_right(at),
            Motion::NextWord => {
                let mut at = at;
                while self.char_after(at).is_some_and(is_word) {
                    at = self.next(at);
                }
                while self.char_after(at).is_some_and(|c| !is_word(c)) {
                    at = self.next(at);
                }
                at
            }
            Motion::LineStart => {
                let start = self.buf.line_start(line);
                let text = self.buf.line_text(line);
                let first = start + (text.len() - text.trim_start().len());
                if at == first {
                    start
                } else {
                    first
                }
            }
            Motion::LineEnd => self.buf.line_end(line),
            Motion::Up | Motion::Down => {
                let col = *goal.get_or_insert_with(|| self.col_of(at));
                if m == Motion::Up {
                    if line == 0 {
                        0
                    } else {
                        self.at_col(line - 1, col)
                    }
                } else if line + 1 >= self.buf.line_count() {
                    self.buf.len()
                } else {
                    self.at_col(line + 1, col)
                }
            }
            Motion::DocStart => 0,
            Motion::DocEnd => self.buf.len(),
        }
    }

    /// Move every cursor; with `extend`, the anchors stay (shift+arrow).
    pub fn move_cursors(&mut self, m: Motion, extend: bool) {
        self.goal.resize(self.buf.sels.len(), None);
        let mut goals = std::mem::take(&mut self.goal);
        let sels: Vec<Sel> = self
            .buf
            .sels
            .iter()
            .zip(goals.iter_mut())
            .map(|(s, g)| {
                // Collapsing a selection with left/right goes to its edge.
                if !extend && !s.is_empty() && matches!(m, Motion::Left | Motion::Right) {
                    return Sel::cursor(if m == Motion::Left { s.start() } else { s.end() });
                }
                let head = self.moved(s.head, m, g);
                if extend {
                    Sel::new(s.anchor, head)
                } else {
                    Sel::cursor(head)
                }
            })
            .collect();
        self.buf.set_sels(sels);
        goals.truncate(self.buf.sels.len());
        self.goal = goals;
    }

    /// Replace every selection with `text`, cursors after it.
    pub fn replace_selections(&mut self, text: &str, kind: EditKind, now_ms: u64) {
        let changes: Vec<Change> = self
            .buf
            .sels
            .iter()
            .map(|s| Change { start: s.start(), end: s.end(), text: text.to_string() })
            .collect();
        let mut delta = 0isize;
        let sels = changes
            .iter()
            .map(|c| {
                let at = (c.start as isize + delta) as usize + text.len();
                delta += text.len() as isize - (c.end - c.start) as isize;
                Sel::cursor(at)
            })
            .collect();
        self.buf.apply(changes, Some(sels), kind, now_ms);
        self.goal.clear();
    }

    /// Text typed by the user: handles auto-pairing, wrapping selections and typing over closers.
    pub fn type_text(&mut self, text: &str, now_ms: u64) {
        let mut chars = text.chars();
        let single = chars.next().filter(|_| chars.next().is_none());
        let Some(c) = single else { return self.replace_selections(text, EditKind::Typing, now_ms) };
        let pair = PAIRS
            .iter()
            .filter(|_| self.settings.auto_pair_brackets)
            .chain(MD_PAIRS.iter().filter(|_| self.settings.auto_pair_markdown))
            .find(|(o, _)| *o == c)
            .copied();
        let is_closer = PAIRS.iter().chain(MD_PAIRS).any(|(_, cl)| *cl == c);
        let any_selection = self.buf.sels.iter().any(|s| !s.is_empty());

        // Wrap selections: typing `*` over selected text yields `*text*`.
        if let (Some((o, cl)), true) = (pair, any_selection) {
            let mut changes = Vec::new();
            let mut sels = Vec::new();
            let mut delta = 0;
            for s in &self.buf.sels {
                if s.is_empty() {
                    changes.push(Change { start: s.head, end: s.head, text: c.to_string() });
                    delta += c.len_utf8();
                    sels.push(Sel::cursor(s.head + delta));
                } else {
                    changes.push(Change { start: s.start(), end: s.start(), text: o.to_string() });
                    changes.push(Change { start: s.end(), end: s.end(), text: cl.to_string() });
                    delta += o.len_utf8();
                    sels.push(Sel::new(s.start() + delta, s.end() + delta));
                    delta += cl.len_utf8();
                }
            }
            self.buf.apply(changes, Some(sels), EditKind::Other, now_ms);
            return;
        }
        if !any_selection {
            // Type over an auto-inserted closer instead of duplicating it.
            if is_closer && self.buf.sels.iter().all(|s| self.char_after(s.head) == Some(c)) {
                let symmetric_md = MD_PAIRS.iter().any(|(o, _)| *o == c);
                if !symmetric_md
                    || self
                        .buf
                        .sels
                        .iter()
                        .all(|s| self.char_before(s.head) != Some(c) || self.char_after(s.head) == Some(c))
                {
                    let sels = self.buf.sels.iter().map(|s| Sel::cursor(self.next(s.head))).collect();
                    self.buf.set_sels(sels);
                    return;
                }
            }
            if let Some((o, cl)) = pair {
                let md = MD_PAIRS.iter().any(|(x, _)| *x == o);
                let ok = self.buf.sels.iter().all(|s| {
                    let after = self.char_after(s.head);
                    let before = self.char_before(s.head);
                    let after_ok = after.is_none_or(|a| {
                        a.is_whitespace() || matches!(a, ')' | ']' | '}' | ',' | '.' | ';' | ':')
                    });
                    // Quotes and Markdown markers pair only at a word start: not inside words
                    // (apostrophes, snake_case) and not when completing `**`.
                    let symmetric = o == cl;
                    let before_ok = !symmetric
                        || before.is_none_or(|b| b.is_whitespace() || matches!(b, '(' | '[' | '{'));
                    after_ok && before_ok && !(md && matches!(o, '=' | '~' | '%' | '$' | '_'))
                });
                if ok {
                    let changes: Vec<Change> = self
                        .buf
                        .sels
                        .iter()
                        .map(|s| Change { start: s.head, end: s.head, text: format!("{o}{cl}") })
                        .collect();
                    let sels = changes
                        .iter()
                        .enumerate()
                        .map(|(i, ch)| {
                            Sel::cursor(ch.start + i * (o.len_utf8() + cl.len_utf8()) + o.len_utf8())
                        })
                        .collect();
                    self.buf.apply(changes, Some(sels), EditKind::Typing, now_ms);
                    return;
                }
            }
        }
        self.replace_selections(text, EditKind::Typing, now_ms);
    }

    fn delete_with(&mut self, now_ms: u64, range: impl Fn(&Editor, usize) -> (usize, usize)) {
        let changes: Vec<Change> = self
            .buf
            .sels
            .iter()
            .map(|s| {
                let (a, b) = if s.is_empty() { range(self, s.head) } else { (s.start(), s.end()) };
                Change { start: a, end: b, text: String::new() }
            })
            .collect();
        self.buf.apply(changes, None, EditKind::Typing, now_ms);
        let sels = self.buf.sels.iter().map(|s| Sel::cursor(s.start())).collect();
        self.buf.set_sels(sels);
        self.goal.clear();
    }

    pub fn delete_backward(&mut self, now_ms: u64) {
        self.delete_with(now_ms, |e, at| {
            let p = e.prev(at);
            // Delete an empty auto-inserted pair as a unit.
            let pairs = PAIRS.iter().chain(MD_PAIRS);
            let both = e.char_before(at).zip(e.char_after(at));
            if both.is_some_and(|(b, a)| pairs.clone().any(|&(o, c)| o == b && c == a)) {
                return (p, e.next(at));
            }
            (p, at)
        });
    }
    pub fn delete_forward(&mut self, now_ms: u64) {
        self.delete_with(now_ms, |e, at| (at, e.next(at)));
    }
    pub fn delete_word_backward(&mut self, now_ms: u64) {
        self.delete_with(now_ms, |e, at| (e.word_left(at), at));
    }
    pub fn delete_word_forward(&mut self, now_ms: u64) {
        self.delete_with(now_ms, |e, at| (at, e.word_right(at)));
    }

    /// Enter: continues list items, task items and quotes; an empty item ends the list.
    pub fn newline(&mut self, now_ms: u64) {
        if !self.settings.smart_lists || self.buf.sels.iter().any(|s| !s.is_empty()) {
            return self.replace_selections("\n", EditKind::Other, now_ms);
        }
        let mut changes = Vec::new();
        for s in &self.buf.sels {
            let line = self.buf.line_of(s.head);
            let (ls, le) = (self.buf.line_start(line), self.buf.line_end(line));
            let text = self.buf.line_text(line).into_owned();
            let in_code = self.buf.doc().block_index_at(s.head).is_some_and(|i| {
                matches!(self.buf.doc().blocks[i].kind(), md_engine::NodeKind::CodeBlock { .. })
            });
            let indent: String = text.chars().take_while(|c| matches!(c, ' ' | '\t')).collect();
            match list_prefix(&text).filter(|_| !in_code) {
                Some((_, _, true)) if s.head == le => {
                    // Empty item: remove the marker instead of adding another.
                    changes.push(Change { start: ls, end: le, text: String::new() });
                }
                Some((ind, marker, _))
                    if s.head - ls >= ind.len() + marker.trim_end().len().min(text.len()) =>
                {
                    changes.push(Change { start: s.head, end: s.head, text: format!("\n{ind}{marker}") });
                }
                _ => {
                    let keep = if s.head - ls >= indent.len() { indent } else { String::new() };
                    changes.push(Change { start: s.head, end: s.head, text: format!("\n{keep}") });
                }
            }
        }
        self.buf.apply(changes, None, EditKind::Other, now_ms);
        let sels = self.buf.sels.iter().map(|s| Sel::cursor(s.end())).collect();
        self.buf.set_sels(sels);
        self.goal.clear();
    }

    fn selected_lines(&self) -> Vec<usize> {
        let mut lines = Vec::new();
        for s in &self.buf.sels {
            let (a, mut b) = (self.buf.line_of(s.start()), self.buf.line_of(s.end()));
            // A selection ending at column 0 does not include that line.
            if b > a && s.end() == self.buf.line_start(b) {
                b -= 1;
            }
            lines.extend(a..=b);
        }
        lines.sort_unstable();
        lines.dedup();
        lines
    }

    fn indent_unit(&self) -> String {
        if self.settings.use_tabs {
            "\t".to_string()
        } else {
            " ".repeat(self.settings.tab_size)
        }
    }

    /// Tab: indent the selected lines (or the current list item); otherwise insert a tab.
    pub fn indent(&mut self, now_ms: u64) {
        let single_cursor_plain =
            self.buf.sels.iter().all(|s| {
                s.is_empty() && list_prefix(&self.buf.line_text(self.buf.line_of(s.head))).is_none()
            });
        if single_cursor_plain {
            let unit = self.indent_unit();
            return self.replace_selections(&unit, EditKind::Typing, now_ms);
        }
        let unit = self.indent_unit();
        let changes = self
            .selected_lines()
            .into_iter()
            .map(|l| Change {
                start: self.buf.line_start(l),
                end: self.buf.line_start(l),
                text: unit.clone(),
            })
            .collect();
        self.buf.apply(changes, None, EditKind::Other, now_ms);
    }

    /// Shift+Tab: remove one level of indentation from the selected lines.
    pub fn outdent(&mut self, now_ms: u64) {
        let tab = self.settings.tab_size;
        let changes = self
            .selected_lines()
            .into_iter()
            .filter_map(|l| {
                let text = self.buf.line_text(l);
                let n = if text.starts_with('\t') {
                    1
                } else {
                    text.bytes().take(tab).take_while(|&b| b == b' ').count()
                };
                (n > 0).then(|| Change {
                    start: self.buf.line_start(l),
                    end: self.buf.line_start(l) + n,
                    text: String::new(),
                })
            })
            .collect();
        self.buf.apply(changes, None, EditKind::Other, now_ms);
    }

    /// Toggle an inline marker (`**`, `*`, `==`, `~~`, `` ` ``) around each selection, or around
    /// the word under a bare cursor.
    pub fn toggle_wrap(&mut self, marker: &str, now_ms: u64) {
        let m = marker.len();
        let mut changes = Vec::new();
        let mut sels = Vec::new();
        let mut delta: isize = 0;
        for s in &self.buf.sels {
            let (mut a, mut b) = (s.start(), s.end());
            if a == b {
                (a, b) = (self.word_left_same_line(a), self.word_right_same_line(b));
            }
            let before = self.buf.slice(a.saturating_sub(m), a) == marker;
            let after = self.buf.slice(b, (b + m).min(self.buf.len())) == marker;
            let inner = self.buf.slice(a, b);
            if before && after {
                changes.push(Change { start: a - m, end: a, text: String::new() });
                changes.push(Change { start: b, end: b + m, text: String::new() });
                delta -= m as isize;
                sels.push(Sel::new((a as isize + delta) as usize, (b as isize + delta) as usize));
                delta -= m as isize;
            } else if inner.len() >= 2 * m && inner.starts_with(marker) && inner.ends_with(marker) {
                changes.push(Change { start: a, end: a + m, text: String::new() });
                changes.push(Change { start: b - m, end: b, text: String::new() });
                sels.push(Sel::new((a as isize + delta) as usize, (b as isize + delta) as usize - 2 * m));
                delta -= 2 * m as isize;
            } else {
                changes.push(Change { start: a, end: a, text: marker.to_string() });
                changes.push(Change { start: b, end: b, text: marker.to_string() });
                delta += m as isize;
                sels.push(Sel::new((a as isize + delta) as usize, (b as isize + delta) as usize));
                delta += m as isize;
            }
        }
        self.buf.apply(changes, Some(sels), EditKind::Other, now_ms);
    }

    fn word_left_same_line(&self, mut at: usize) -> usize {
        while self.char_before(at).is_some_and(is_word) {
            at = self.prev(at);
        }
        at
    }
    fn word_right_same_line(&self, mut at: usize) -> usize {
        while self.char_after(at).is_some_and(is_word) {
            at = self.next(at);
        }
        at
    }

    /// Rewrites the start of each selected line with `f(line) -> (bytes to remove, text to insert)`.
    fn edit_line_starts(&mut self, now_ms: u64, f: impl Fn(&str) -> Option<(usize, usize, String)>) {
        let changes = self
            .selected_lines()
            .into_iter()
            .filter_map(|l| {
                let text = self.buf.line_text(l);
                let (from, to, ins) = f(&text)?;
                let ls = self.buf.line_start(l);
                Some(Change { start: ls + from, end: ls + to, text: ins })
            })
            .collect();
        self.buf.apply(changes, None, EditKind::Other, now_ms);
    }

    /// Cycle a line: plain text -> `- [ ]` -> `- [x]` -> `- [ ]`.
    pub fn toggle_task(&mut self, now_ms: u64) {
        self.edit_line_starts(now_ms, |line| {
            let ind = line.len() - line.trim_start().len();
            let rest = &line[ind..];
            let rb = rest.as_bytes();
            let bullet = rb.len() >= 2 && matches!(rb[0], b'-' | b'*' | b'+') && rb[1] == b' ';
            if bullet && rb.len() >= 5 && rb[2] == b'[' && rb[4] == b']' {
                let next = if rb[3] == b' ' { "x" } else { " " };
                Some((ind + 3, ind + 4, next.to_string()))
            } else if bullet {
                Some((ind + 2, ind + 2, "[ ] ".to_string()))
            } else {
                Some((ind, ind, "- [ ] ".to_string()))
            }
        });
    }

    /// Set the heading level of the selected lines (0 removes the heading).
    pub fn set_heading(&mut self, level: usize, now_ms: u64) {
        self.edit_line_starts(now_ms, |line| {
            let hashes = line.bytes().take_while(|&b| b == b'#').count();
            let existing =
                if hashes > 0 && hashes <= 6 && line[hashes..].starts_with(' ') { hashes + 1 } else { 0 };
            let ins = if level == 0 { String::new() } else { format!("{} ", "#".repeat(level.min(6))) };
            Some((0, existing, ins))
        });
    }

    pub fn toggle_blockquote(&mut self, now_ms: u64) {
        let all_quoted = self.selected_lines().iter().all(|&l| self.buf.line_text(l).starts_with('>'));
        self.edit_line_starts(now_ms, |line| {
            if all_quoted {
                Some((0, if line.starts_with("> ") { 2 } else { 1 }, String::new()))
            } else {
                Some((0, 0, "> ".to_string()))
            }
        });
    }

    /// Swap the selected lines with the line above (`up`) or below.
    pub fn move_lines(&mut self, up: bool, now_ms: u64) {
        let lines = self.selected_lines();
        let (Some(&first), Some(&last)) = (lines.first(), lines.last()) else { return };
        if (up && first == 0) || (!up && last + 1 >= self.buf.line_count()) {
            return;
        }
        let (a, b) = if up { (first - 1, last) } else { (first, last + 1) };
        let (start, end) = (self.buf.line_start(a), self.buf.line_end(b));
        let block: Vec<String> = (a..=b).map(|l| self.buf.line_text(l).into_owned()).collect();
        let mut rotated = block.clone();
        if up {
            rotated.rotate_left(1);
        } else {
            rotated.rotate_right(1);
        }
        let shift =
            if up { -(block[0].len() as isize + 1) } else { block[block.len() - 1].len() as isize + 1 };
        let sels = self
            .buf
            .sels
            .iter()
            .map(|s| Sel::new((s.anchor as isize + shift) as usize, (s.head as isize + shift) as usize))
            .collect();
        self.buf.apply(
            vec![Change { start, end, text: rotated.join("\n") }],
            Some(sels),
            EditKind::Other,
            now_ms,
        );
    }

    pub fn duplicate_lines(&mut self, now_ms: u64) {
        let lines = self.selected_lines();
        let (Some(&first), Some(&last)) = (lines.first(), lines.last()) else { return };
        let text = self.buf.slice(self.buf.line_start(first), self.buf.line_end(last)).into_owned();
        let at = self.buf.line_end(last);
        self.buf.apply(
            vec![Change { start: at, end: at, text: format!("\n{text}") }],
            None,
            EditKind::Other,
            now_ms,
        );
    }

    pub fn select_all(&mut self) {
        let len = self.buf.len();
        self.buf.set_sels(vec![Sel::new(0, len)]);
    }

    /// Add a cursor on the line above or below each existing cursor (same column).
    pub fn add_cursor_vertical(&mut self, up: bool) {
        let mut sels = self.buf.sels.clone();
        for s in &self.buf.sels {
            let line = self.buf.line_of(s.head);
            let target =
                if up { line.checked_sub(1) } else { (line + 1 < self.buf.line_count()).then_some(line + 1) };
            if let Some(t) = target {
                sels.push(Sel::cursor(self.at_col(t, self.col_of(s.head))));
            }
        }
        self.buf.set_sels(sels);
    }

    /// Ctrl+D: select the word under the cursor, then add the next occurrence of the selection.
    pub fn select_next_occurrence(&mut self) {
        let Some(&last) = self.buf.sels.last() else { return };
        if last.is_empty() {
            let (a, b) = (self.word_left_same_line(last.head), self.word_right_same_line(last.head));
            let mut sels = self.buf.sels.clone();
            if let Some(l) = sels.last_mut() {
                *l = Sel::new(a, b);
            }
            return self.buf.set_sels(sels);
        }
        let needle = self.buf.slice(last.start(), last.end()).into_owned();
        let text = self.buf.text();
        let taken = |p: usize| self.buf.sels.iter().any(|s| s.start() == p);
        let found = text[last.end()..]
            .match_indices(&needle)
            .map(|(p, _)| p + last.end())
            .chain(text[..last.start()].match_indices(&needle).map(|(p, _)| p))
            .find(|&p| !taken(p));
        if let Some(p) = found {
            let mut sels = self.buf.sels.clone();
            sels.push(Sel::new(p, p + needle.len()));
            self.buf.set_sels(sels);
        }
    }

    pub fn undo(&mut self) -> bool {
        self.goal.clear();
        self.buf.undo()
    }
    pub fn redo(&mut self) -> bool {
        self.goal.clear();
        self.buf.redo()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an editor from text where `|` marks cursors and `[`…`]` selections (escaped as `\[`).
    fn ed(marked: &str) -> Editor {
        let mut text = String::new();
        let mut sels = Vec::new();
        let mut open = None;
        for c in marked.chars() {
            match c {
                '‹' => open = Some(text.len()),
                '›' => sels.push(Sel::new(open.take().unwrap(), text.len())),
                '|' => sels.push(Sel::cursor(text.len())),
                c => text.push(c),
            }
        }
        let mut e = Editor::new(&text);
        e.buf.set_sels(sels);
        e
    }

    fn show(e: &Editor) -> String {
        let mut text = e.buf.text();
        let mut marks: Vec<(usize, &str)> = Vec::new();
        for s in &e.buf.sels {
            if s.is_empty() {
                marks.push((s.head, "|"));
            } else {
                marks.push((s.start(), "‹"));
                marks.push((s.end(), "›"));
            }
        }
        marks.sort_by_key(|m| std::cmp::Reverse(m.0));
        for (at, m) in marks {
            text.insert_str(at, m);
        }
        text
    }

    #[test]
    fn multi_cursor_typing_and_deleting() {
        let mut e = ed("a| b| c|");
        e.type_text("X", 0);
        assert_eq!(show(&e), "aX| bX| cX|");
        e.delete_backward(0);
        assert_eq!(show(&e), "a| b| c|");
        e.replace_selections("long", EditKind::Other, 0);
        assert_eq!(show(&e), "along| blong| clong|");
        e.delete_word_backward(0);
        assert_eq!(show(&e), "| | |");
        assert_eq!(e.buf.doc(), &md_engine::parse(&e.buf.text()));
    }

    #[test]
    fn movement() {
        let mut e = ed("  héllo wörld\nsecond line\nx|");
        e.move_cursors(Motion::Up, false);
        assert_eq!(show(&e), "  héllo wörld\ns|econd line\nx");
        e.move_cursors(Motion::LineEnd, false);
        e.move_cursors(Motion::Up, false);
        assert_eq!(show(&e), "  héllo wör|ld\nsecond line\nx");
        e.move_cursors(Motion::Down, false);
        e.move_cursors(Motion::Down, false);
        assert_eq!(show(&e), "  héllo wörld\nsecond line\nx|", "short line clamps, goal column kept");
        e.move_cursors(Motion::Up, false);
        e.move_cursors(Motion::Up, false);
        assert_eq!(show(&e), "  héllo wör|ld\nsecond line\nx");
        e.move_cursors(Motion::WordLeft, false);
        assert_eq!(show(&e), "  héllo |wörld\nsecond line\nx");
        e.move_cursors(Motion::LineStart, false);
        assert_eq!(show(&e), "  |héllo wörld\nsecond line\nx");
        e.move_cursors(Motion::LineStart, false);
        assert_eq!(show(&e), "|  héllo wörld\nsecond line\nx");
        e.move_cursors(Motion::WordRight, true);
        assert_eq!(show(&e), "‹  héllo› wörld\nsecond line\nx");
        e.move_cursors(Motion::Right, false);
        assert_eq!(show(&e), "  héllo| wörld\nsecond line\nx", "collapse to the selection edge");
        e.move_cursors(Motion::DocEnd, true);
        assert_eq!(show(&e), "  héllo‹ wörld\nsecond line\nx›");
    }

    #[test]
    fn auto_pairs() {
        let mut e = ed("|");
        e.type_text("(", 0);
        assert_eq!(show(&e), "(|)");
        e.type_text("[", 0);
        assert_eq!(show(&e), "([|])");
        e.type_text("]", 0);
        assert_eq!(show(&e), "([]|)", "typing the closer steps over it");
        e.delete_backward(0);
        e.delete_backward(0);
        assert_eq!(show(&e), "(|)");
        e.delete_backward(0);
        assert_eq!(show(&e), "|", "an empty pair is deleted together");
        let mut e = ed("don|");
        e.type_text("'", 0);
        assert_eq!(show(&e), "don'|", "no pairing inside a word");
        let mut e = ed("‹word›");
        e.type_text("*", 0);
        assert_eq!(show(&e), "*‹word›*", "typing a marker wraps the selection");
        let mut e = ed("|");
        e.settings.auto_pair_brackets = false;
        e.type_text("(", 0);
        assert_eq!(show(&e), "(|");
    }

    #[test]
    fn smart_lists() {
        let mut e = ed("- item|");
        e.newline(0);
        assert_eq!(show(&e), "- item\n- |");
        e.newline(0);
        assert_eq!(show(&e), "- item\n|", "enter on an empty item ends the list");
        let mut e = ed("  3. third|");
        e.newline(0);
        assert_eq!(show(&e), "  3. third\n  4. |");
        let mut e = ed("- [x] done|");
        e.newline(0);
        assert_eq!(show(&e), "- [x] done\n- [ ] |", "new task starts unchecked");
        let mut e = ed("> quote|");
        e.newline(0);
        assert_eq!(show(&e), "> quote\n> |");
        let mut e = ed("\tindented|");
        e.newline(0);
        assert_eq!(show(&e), "\tindented\n\t|");
        let mut e = ed("```\n- not a list|\n```");
        e.newline(0);
        assert_eq!(show(&e), "```\n- not a list\n|\n```", "no list continuation inside code");
        let mut e = ed("- split| here");
        e.newline(0);
        assert_eq!(show(&e), "- split\n- | here");
    }

    #[test]
    fn indent_outdent() {
        let mut e = ed("- a|\n- b");
        e.indent(0);
        assert_eq!(show(&e), "\t- a|\n- b");
        e.outdent(0);
        assert_eq!(show(&e), "- a|\n- b");
        let mut e = ed("‹one\ntwo›\nthree");
        e.settings.use_tabs = false;
        e.settings.tab_size = 2;
        e.indent(0);
        assert_eq!(e.buf.text(), "  one\n  two\nthree");
        e.outdent(0);
        e.outdent(0);
        assert_eq!(e.buf.text(), "one\ntwo\nthree");
        let mut e = ed("plain|");
        e.indent(0);
        assert_eq!(show(&e), "plain\t|");
    }

    #[test]
    fn formatting_toggles() {
        let mut e = ed("make ‹this› bold");
        e.toggle_wrap("**", 0);
        assert_eq!(show(&e), "make **‹this›** bold");
        e.toggle_wrap("**", 0);
        assert_eq!(show(&e), "make ‹this› bold");
        let mut e = ed("a wo|rd b");
        e.toggle_wrap("==", 0);
        assert_eq!(e.buf.text(), "a ==word== b");
        let mut e = ed("‹**x**›");
        e.toggle_wrap("**", 0);
        assert_eq!(show(&e), "‹x›");
        let mut e = ed("text|\n- item");
        e.toggle_task(0);
        assert_eq!(e.buf.text(), "- [ ] text\n- item");
        e.toggle_task(0);
        assert_eq!(e.buf.text(), "- [x] text\n- item");
        e.toggle_task(0);
        assert_eq!(e.buf.text(), "- [ ] text\n- item");
        let mut e = ed("## Title|");
        e.set_heading(1, 0);
        assert_eq!(e.buf.text(), "# Title");
        e.set_heading(0, 0);
        assert_eq!(e.buf.text(), "Title");
        e.toggle_blockquote(0);
        assert_eq!(e.buf.text(), "> Title");
        e.toggle_blockquote(0);
        assert_eq!(e.buf.text(), "Title");
    }

    #[test]
    fn line_operations() {
        let mut e = ed("one\ntw|o\nthree");
        e.move_lines(true, 0);
        assert_eq!(show(&e), "tw|o\none\nthree");
        e.move_lines(false, 0);
        e.move_lines(false, 0);
        assert_eq!(show(&e), "one\nthree\ntw|o");
        e.move_lines(false, 0);
        assert_eq!(show(&e), "one\nthree\ntw|o", "already last");
        e.duplicate_lines(0);
        assert_eq!(e.buf.text(), "one\nthree\ntwo\ntwo");
    }

    #[test]
    fn extra_cursors() {
        let mut e = ed("foo bar\nfo|o baz\nfoo");
        e.select_next_occurrence();
        assert_eq!(show(&e), "foo bar\n‹foo› baz\nfoo");
        e.select_next_occurrence();
        e.select_next_occurrence();
        assert_eq!(show(&e), "‹foo› bar\n‹foo› baz\n‹foo›", "wraps around");
        e.type_text("x", 0);
        assert_eq!(e.buf.text(), "x bar\nx baz\nx");
        let mut e = ed("abc\nab|c\nabc");
        e.add_cursor_vertical(true);
        e.add_cursor_vertical(false);
        assert_eq!(show(&e), "ab|c\nab|c\nab|c");
        e.select_all();
        assert_eq!(e.buf.sels.len(), 1);
    }

    #[test]
    fn undo_restores_cursors() {
        let mut e = ed("a| b|");
        e.type_text("1", 0);
        e.newline(0);
        assert!(e.undo() && e.undo());
        assert_eq!(show(&e), "a| b|");
        assert!(e.redo());
        assert_eq!(show(&e), "a1| b1|");
    }
}
