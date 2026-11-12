//! Vim emulation: a deliberately small, documented subset.
//!
//! Normal mode: counts, `h j k l w b e 0 ^ $ gg G`, `i a I A o O`, `x`, `dd`, `yy`, `cc`,
//! `d`/`c`/`y` + motion (`w b e $ 0 h l`), `D`, `C`, `p`, `P`, `u`, `Ctrl-r` (as `<C-r>`), `v`,
//! `V`, `J`. Visual mode: motions, `d x y c`, `Esc`. Insert mode: `Esc`.
//! Not implemented: registers other than the unnamed one, text objects, `.` repeat, marks,
//! macros, search (`/`), ex commands.

use crate::buffer::{Change, EditKind, Sel};
use crate::editor::{Editor, Motion};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VimMode {
    Normal,
    Insert,
    Visual,
    VisualLine,
}

pub struct Vim {
    pub mode: VimMode,
    count: usize,
    pending: Option<char>,
    register: String,
    register_linewise: bool,
}

impl Default for Vim {
    fn default() -> Self {
        Vim {
            mode: VimMode::Normal,
            count: 0,
            pending: None,
            register: String::new(),
            register_linewise: false,
        }
    }
}

impl Vim {
    fn motion(key: &str) -> Option<Motion> {
        Some(match key {
            "h" => Motion::Left,
            "l" => Motion::Right,
            "j" => Motion::Down,
            "k" => Motion::Up,
            "w" => Motion::NextWord,
            "e" => Motion::WordRight,
            "b" => Motion::WordLeft,
            "0" => Motion::LineStart,
            "^" => Motion::LineStart,
            "$" => Motion::LineEnd,
            "G" => Motion::DocEnd,
            _ => return None,
        })
    }

    fn line_range(e: &Editor, count: usize) -> (usize, usize) {
        let first = e.buf.line_of(e.buf.sels[0].head);
        let last = (first + count.max(1) - 1).min(e.buf.line_count().saturating_sub(1));
        let start = e.buf.line_start(first);
        let end = if last + 1 < e.buf.line_count() { e.buf.line_start(last + 1) } else { e.buf.len() };
        (start, end)
    }

    fn yank(&mut self, e: &Editor, start: usize, end: usize, linewise: bool) {
        self.register = e.buf.slice(start, end).into_owned();
        if linewise && !self.register.ends_with('\n') {
            self.register.push('\n');
        }
        self.register_linewise = linewise;
    }

    fn delete(&mut self, e: &mut Editor, start: usize, end: usize, linewise: bool, now: u64) {
        self.yank(e, start, end, linewise);
        // Deleting the last line also removes the newline before it.
        let start = if linewise && end == e.buf.len() && start > 0 && !e.buf.slice(start, end).ends_with('\n')
        {
            start - 1
        } else {
            start
        };
        e.buf.apply(
            vec![Change { start, end, text: String::new() }],
            Some(vec![Sel::cursor(start)]),
            EditKind::Other,
            now,
        );
    }

    /// Feed one key. Printable keys are passed as themselves; special keys as `<Esc>`, `<C-r>`.
    /// Returns `true` if the key was consumed (in insert mode, text keys are not).
    pub fn key(&mut self, e: &mut Editor, key: &str, now: u64) -> bool {
        if key == "<Esc>" {
            if self.mode == VimMode::Insert {
                e.move_cursors(Motion::Left, false);
            }
            let sels = e.buf.sels.iter().map(|s| Sel::cursor(s.head)).collect();
            e.buf.set_sels(sels);
            self.mode = VimMode::Normal;
            self.count = 0;
            self.pending = None;
            return true;
        }
        if self.mode == VimMode::Insert {
            return false;
        }
        if let Some(d) = key.chars().next().filter(|c| c.is_ascii_digit() && key.len() == 1) {
            if d != '0' || self.count > 0 {
                self.count = self.count * 10 + d as usize - '0' as usize;
                return true;
            }
        }
        let count = self.count.max(1);
        let had_count = self.count > 0;
        self.count = 0;
        let visual = matches!(self.mode, VimMode::Visual | VimMode::VisualLine);

        if let Some(op) = self.pending.take() {
            if op == 'g' {
                if key == "g" {
                    e.move_cursors(Motion::DocStart, visual);
                }
                return true;
            }
            let at = e.buf.sels[0].head;
            let (start, end, linewise) = if key.len() == 1 && key.starts_with(op) {
                let (s, en) = Self::line_range(e, count);
                (s, en, true)
            } else if let Some(m) = Self::motion(key) {
                for _ in 0..count {
                    e.move_cursors(m, true);
                }
                let s = e.buf.sels[0];
                e.buf.set_sels(vec![Sel::cursor(at)]);
                (s.start(), s.end(), false)
            } else {
                return true;
            };
            match op {
                'y' => self.yank(e, start, end, linewise),
                'd' => self.delete(e, start, end, linewise, now),
                _ => {
                    // `c`: linewise change keeps the line itself.
                    let end = if linewise && e.buf.slice(start, end).ends_with('\n') { end - 1 } else { end };
                    self.delete(e, start, end, false, now);
                    self.mode = VimMode::Insert;
                }
            }
            return true;
        }

        if visual {
            let s = e.buf.sels[0];
            let (start, end) = if self.mode == VimMode::VisualLine {
                let (a, b) = (e.buf.line_of(s.start()), e.buf.line_of(s.end()));
                (
                    e.buf.line_start(a),
                    if b + 1 < e.buf.line_count() { e.buf.line_start(b + 1) } else { e.buf.len() },
                )
            } else {
                (s.start(), (s.end() + 1).min(e.buf.len()))
            };
            let linewise = self.mode == VimMode::VisualLine;
            match key {
                "d" | "x" => {
                    self.delete(e, start, end, linewise, now);
                    self.mode = VimMode::Normal;
                }
                "y" => {
                    self.yank(e, start, end, linewise);
                    e.buf.set_sels(vec![Sel::cursor(start)]);
                    self.mode = VimMode::Normal;
                }
                "c" => {
                    self.delete(e, start, end, false, now);
                    self.mode = VimMode::Insert;
                }
                "g" => self.pending = Some('g'),
                _ => {
                    if let Some(m) = Self::motion(key) {
                        for _ in 0..count {
                            e.move_cursors(m, true);
                        }
                    }
                }
            }
            return true;
        }

        match key {
            "i" => self.mode = VimMode::Insert,
            "a" => {
                e.move_cursors(Motion::Right, false);
                self.mode = VimMode::Insert;
            }
            "I" => {
                e.move_cursors(Motion::LineEnd, false);
                e.move_cursors(Motion::LineStart, false);
                self.mode = VimMode::Insert;
            }
            "A" => {
                e.move_cursors(Motion::LineEnd, false);
                self.mode = VimMode::Insert;
            }
            "o" => {
                e.move_cursors(Motion::LineEnd, false);
                e.replace_selections("\n", EditKind::Other, now);
                self.mode = VimMode::Insert;
            }
            "O" => {
                let at = e.buf.line_start(e.buf.line_of(e.buf.sels[0].head));
                e.buf.apply(
                    vec![Change { start: at, end: at, text: "\n".into() }],
                    Some(vec![Sel::cursor(at)]),
                    EditKind::Other,
                    now,
                );
                self.mode = VimMode::Insert;
            }
            "x" => {
                let at = e.buf.sels[0].head;
                let line_end = e.buf.line_end(e.buf.line_of(at));
                let mut end = at;
                for _ in 0..count {
                    end += e.buf.slice(end, line_end).chars().next().map_or(0, char::len_utf8);
                }
                if end > at {
                    self.delete(e, at, end, false, now);
                }
            }
            "D" | "C" => {
                let at = e.buf.sels[0].head;
                let end = e.buf.line_end(e.buf.line_of(at));
                self.delete(e, at, end, false, now);
                if key == "C" {
                    self.mode = VimMode::Insert;
                }
            }
            "d" | "y" | "c" | "g" => {
                self.pending = key.chars().next();
                self.count = if had_count { count } else { 0 };
            }
            "p" | "P" => {
                let at = e.buf.sels[0].head;
                let text = self.register.repeat(count);
                let pos = if self.register_linewise {
                    let line = e.buf.line_of(at);
                    if key == "P" {
                        e.buf.line_start(line)
                    } else if line + 1 < e.buf.line_count() {
                        e.buf.line_start(line + 1)
                    } else {
                        e.buf.len()
                    }
                } else if key == "P" || e.buf.is_empty() {
                    at
                } else {
                    (at + e.buf.slice(at, e.buf.len().min(at + 4)).chars().next().map_or(0, char::len_utf8))
                        .min(e.buf.len())
                };
                let needs_nl = self.register_linewise
                    && pos == e.buf.len()
                    && pos > 0
                    && !e.buf.slice(pos - 1, pos).ends_with('\n');
                let text = if needs_nl { format!("\n{}", text.trim_end_matches('\n')) } else { text };
                let cursor = if self.register_linewise {
                    pos + needs_nl as usize
                } else {
                    pos + text.len().saturating_sub(1)
                };
                e.buf.apply(
                    vec![Change { start: pos, end: pos, text }],
                    Some(vec![Sel::cursor(cursor)]),
                    EditKind::Other,
                    now,
                );
            }
            "u" => {
                for _ in 0..count {
                    e.undo();
                }
            }
            "<C-r>" => {
                for _ in 0..count {
                    e.redo();
                }
            }
            "v" => self.mode = VimMode::Visual,
            "V" => self.mode = VimMode::VisualLine,
            "J" => {
                let line = e.buf.line_of(e.buf.sels[0].head);
                if line + 1 < e.buf.line_count() {
                    let (a, b) = (e.buf.line_end(line), e.buf.line_start(line + 1));
                    let next = e.buf.line_text(line + 1);
                    let skip = next.len() - next.trim_start().len();
                    e.buf.apply(
                        vec![Change { start: a, end: b + skip, text: " ".into() }],
                        Some(vec![Sel::cursor(a)]),
                        EditKind::Other,
                        now,
                    );
                }
            }
            "G" if had_count => {
                let at = e.buf.line_start(count - 1);
                e.buf.set_sels(vec![Sel::cursor(at)]);
            }
            _ => {
                if let Some(m) = Self::motion(key) {
                    for _ in 0..count {
                        e.move_cursors(m, false);
                    }
                    if key == "0" {
                        let at = e.buf.line_start(e.buf.line_of(e.buf.sels[0].head));
                        e.buf.set_sels(vec![Sel::cursor(at)]);
                    }
                } else {
                    return key.chars().count() == 1;
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, at: usize, keys: &[&str]) -> (String, usize, VimMode) {
        let mut e = Editor::new(text);
        e.buf.set_sels(vec![Sel::cursor(at)]);
        let mut v = Vim::default();
        for k in keys {
            if !v.key(&mut e, k, 0) {
                e.type_text(k, 0);
            }
        }
        (e.buf.text(), e.buf.sels[0].head, v.mode)
    }

    #[test]
    fn motions_and_counts() {
        assert_eq!(run("one two three\nx", 0, &["2", "w"]).1, 8);
        assert_eq!(run("one two three\nx", 0, &["2", "e"]).1, 7);
        assert_eq!(run("one two three\nx", 0, &["$"]).1, 13);
        assert_eq!(run("a\nb\nc", 0, &["G"]).1, 5);
        assert_eq!(run("a\nb\nc", 4, &["g", "g"]).1, 0);
        assert_eq!(run("a\nb\nc", 0, &["3", "G"]).1, 4);
        assert_eq!(run("  x", 3, &["0"]).1, 0);
    }

    #[test]
    fn insert_and_escape() {
        let (t, at, m) = run("ab", 1, &["i", "X", "Y", "<Esc>"]);
        assert_eq!((t.as_str(), at, m), ("aXYb", 2, VimMode::Normal));
        assert_eq!(run("ab", 0, &["A", "!", "<Esc>"]).0, "ab!");
        assert_eq!(run("ab\ncd", 0, &["o", "x", "<Esc>"]).0, "ab\nx\ncd");
        assert_eq!(run("ab\ncd", 4, &["O", "x", "<Esc>"]).0, "ab\nx\ncd");
    }

    #[test]
    fn operators() {
        assert_eq!(run("one\ntwo\nthree", 5, &["d", "d"]).0, "one\nthree");
        assert_eq!(run("one\ntwo\nthree", 0, &["2", "d", "d"]).0, "three");
        assert_eq!(run("one\ntwo", 5, &["d", "d"]).0, "one");
        assert_eq!(run("one two three", 4, &["d", "w"]).0, "one three");
        assert_eq!(run("one two three", 4, &["d", "e"]).0, "one  three");
        assert_eq!(run("one two", 4, &["D"]).0, "one ");
        assert_eq!(run("abc", 0, &["2", "x"]).0, "c");
        assert_eq!(run("one\ntwo", 0, &["y", "y", "p"]).0, "one\none\ntwo");
        assert_eq!(run("one\ntwo", 4, &["y", "y", "p"]).0, "one\ntwo\ntwo");
        assert_eq!(run("one\ntwo", 4, &["d", "d", "P"]).0, "two\none");
        assert_eq!(run("ab", 0, &["x", "p"]).0, "ba");
        let (t, _, m) = run("one\ntwo", 0, &["c", "c", "N", "<Esc>"]);
        assert_eq!((t.as_str(), m), ("N\ntwo", VimMode::Normal));
        assert_eq!(run("a\n  b", 0, &["J"]).0, "a b");
    }

    #[test]
    fn visual_and_undo() {
        assert_eq!(run("abcdef", 1, &["v", "l", "l", "d"]).0, "aef");
        assert_eq!(run("one\ntwo\nthree", 0, &["V", "j", "d"]).0, "three");
        assert_eq!(run("abc", 0, &["v", "l", "y", "$", "p"]).0, "abcab");
        assert_eq!(run("one\ntwo", 0, &["d", "d", "u"]).0, "one\ntwo");
        assert_eq!(run("one\ntwo", 0, &["d", "d", "u", "<C-r>"]).0, "two");
    }
}
