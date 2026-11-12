//! Editor features layered over `editor-core`: folding, table editing and vim keys.

use crate::shell::{App, Tab, ViewMode};
use editor_core::fold::fold_ranges;
use editor_core::table::Table;
use editor_core::vim::VimMode;
use editor_core::{Change, EditKind, Sel};
use md_engine::{Align, NodeKind};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TableOp {
    /// Tab and Shift+Tab. Moving past the last cell appends a row.
    NextCell(bool),
    /// Enter: the same column one row down, appending a row at the end.
    NextRow,
    RowBefore,
    RowAfter,
    RowUp,
    RowDown,
    RowDelete,
    ColBefore,
    ColAfter,
    ColLeft,
    ColRight,
    ColDelete,
    Align(Align),
    Sort(bool),
    /// Rewrite the table with aligned columns.
    Format,
}

pub fn table_op(command: &str) -> Option<TableOp> {
    Some(match command.strip_prefix("editor:table-")? {
        "row-before" => TableOp::RowBefore,
        "row-after" => TableOp::RowAfter,
        "row-up" => TableOp::RowUp,
        "row-down" => TableOp::RowDown,
        "row-delete" => TableOp::RowDelete,
        "col-before" => TableOp::ColBefore,
        "col-after" => TableOp::ColAfter,
        "col-left" => TableOp::ColLeft,
        "col-right" => TableOp::ColRight,
        "col-delete" => TableOp::ColDelete,
        "col-align-left" => TableOp::Align(Align::Left),
        "col-align-center" => TableOp::Align(Align::Center),
        "col-align-right" => TableOp::Align(Align::Right),
        "sort-ascending" => TableOp::Sort(false),
        "sort-descending" => TableOp::Sort(true),
        "format" => TableOp::Format,
        _ => return None,
    })
}

fn unescaped_pipes(s: &str) -> usize {
    let b = s.as_bytes();
    (0..b.len()).filter(|&i| b[i] == b'|' && (i == 0 || b[i - 1] != b'\\')).count()
}

impl Tab {
    /// The table under the primary cursor: source range, parsed grid, and the cursor's cell.
    fn table_at_cursor(&self) -> Option<(usize, usize, Table, usize, usize)> {
        let buf = &self.editor.buf;
        let head = buf.sels.first()?.head;
        let doc = buf.doc();
        let b = &doc.blocks[doc.block_index_at(head)?];
        if !matches!(b.kind(), NodeKind::Table { .. }) {
            return None;
        }
        let root = b.tree.root();
        let (s, e) = (b.start + root.span.start as usize, b.start + root.span.end as usize);
        if head < s || head > e {
            return None;
        }
        let src = buf.slice(s, e);
        let table = Table::parse(&src)?;
        let before = src.get(..head - s)?;
        let line = before.matches('\n').count();
        let line_start = before.rfind('\n').map_or(0, |p| p + 1);
        let leading = src[line_start..].trim_start().starts_with('|') as usize;
        let col = unescaped_pipes(&before[line_start..]).saturating_sub(leading).min(table.cols() - 1);
        // Line 1 is the delimiter row; it counts as the header.
        let row = line.saturating_sub(1).min(table.rows.len() - 1);
        Some((s, e, table, row, col))
    }

    pub fn cursor_in_table(&self) -> bool {
        self.table_at_cursor().is_some()
    }

    /// Apply a table operation at the cursor. Returns false when the cursor is not in a table.
    pub fn table_op(&mut self, op: TableOp, now: u64) -> bool {
        let Some((s, e, mut t, mut row, mut col)) = self.table_at_cursor() else { return false };
        match op {
            TableOp::NextCell(forward) => (row, col) = t.next_cell(row, col, forward),
            TableOp::NextRow => {
                if row + 1 >= t.rows.len() {
                    t.insert_row(t.rows.len());
                }
                row += 1;
            }
            TableOp::RowBefore => {
                // Nothing goes above the header.
                row = row.max(1);
                t.insert_row(row);
            }
            TableOp::RowAfter => {
                row += 1;
                t.insert_row(row);
            }
            TableOp::RowUp if row > 1 => {
                t.move_row(row, row - 1);
                row -= 1;
            }
            TableOp::RowDown if row > 0 && row + 1 < t.rows.len() => {
                t.move_row(row, row + 1);
                row += 1;
            }
            TableOp::RowDelete if row > 0 => {
                t.delete_row(row);
                row = row.min(t.rows.len() - 1);
            }
            TableOp::ColBefore => t.insert_col(col),
            TableOp::ColAfter => {
                col += 1;
                t.insert_col(col);
            }
            TableOp::ColLeft if col > 0 => {
                t.move_col(col, col - 1);
                col -= 1;
            }
            TableOp::ColRight if col + 1 < t.cols() => {
                t.move_col(col, col + 1);
                col += 1;
            }
            TableOp::ColDelete => {
                t.delete_col(col);
                col = col.min(t.cols() - 1);
            }
            TableOp::Align(a) => t.aligns[col] = a,
            TableOp::Sort(descending) => t.sort_by(col, descending),
            _ => {}
        }
        let md = t.to_markdown();
        let text = md.trim_end_matches('\n').to_string();
        // `cell_offset` points just after "| "; aligned cells are padded on the left.
        let cell = t.cell_offset(row, col);
        let pad = md[cell.min(md.len())..].bytes().take_while(|b| *b == b' ').count();
        let content = &t.rows[row][col];
        let at = s + cell + if content.is_empty() { 0 } else { pad + content.len() };
        if *self.editor.buf.slice(s, e) == *text {
            // Nothing to rewrite: only the cursor moves, and no undo step is recorded.
            self.editor.buf.set_sels(vec![Sel::cursor(at)]);
            return true;
        }
        self.editor.buf.apply(
            vec![Change { start: s, end: e, text }],
            Some(vec![Sel::cursor(at)]),
            EditKind::Other,
            now,
        );
        self.last_edit_ms = now;
        true
    }

    /// Insert an empty 2x2 table on its own lines after the cursor's line.
    pub fn insert_table(&mut self, now: u64) {
        let buf = &self.editor.buf;
        let head = buf.sels.first().map_or(0, |s| s.head);
        let line = buf.line_of(head);
        let (ls, le) = (buf.line_start(line), buf.line_end(line));
        let t = Table::new(2, 2);
        let md = t.to_markdown();
        // After a non-empty line the table starts a new paragraph and reuses that line's newline.
        let (at, prefix, md) = if buf.slice(ls, le).trim().is_empty() {
            (ls, "", md.as_str())
        } else {
            (le, "\n\n", md.trim_end_matches('\n'))
        };
        let cursor = at + prefix.len() + t.cell_offset(0, 0);
        self.editor.buf.apply(
            vec![Change { start: at, end: at, text: format!("{prefix}{md}") }],
            Some(vec![Sel::cursor(cursor)]),
            EditKind::Other,
            now,
        );
        self.last_edit_ms = now;
    }

    // ---- folding ----

    /// Bring fold anchors and the hidden ranges up to date with the buffer.
    pub fn refresh_folds(&mut self) {
        match self.editor.buf.take_changes() {
            Some(steps) => steps.iter().for_each(|s| self.folds.map(s)),
            None => self.folds.clear(),
        }
        let v = self.editor.buf.version;
        if self.folds.is_empty() {
            self.hidden = (v, Vec::new());
        } else if self.hidden.0 != v || self.hidden_dirty {
            let text = self.editor.buf.text();
            let ranges = fold_ranges(self.editor.buf.doc(), &text);
            self.hidden = (v, self.folds.hidden(&ranges));
        }
        self.hidden_dirty = false;
    }

    /// Fold or unfold at the cursor: the foldable line the cursor is on, otherwise the
    /// innermost section around it.
    pub fn toggle_fold_at_cursor(&mut self) {
        let head = self.editor.buf.sels.first().map_or(0, |s| s.head);
        let text = self.editor.buf.text();
        let ranges = fold_ranges(self.editor.buf.doc(), &text);
        let on_line = ranges.iter().rfind(|r| r.header <= head && head <= r.start);
        let around =
            ranges.iter().filter(|r| r.start < head && head <= r.end).min_by_key(|r| r.end - r.start);
        if let Some(r) = on_line.or(around) {
            self.toggle_fold(r.header);
        }
    }

    pub fn toggle_fold(&mut self, header: usize) {
        self.refresh_folds();
        self.folds.toggle(header);
        self.hidden_dirty = true;
        self.refresh_folds();
        self.leave_folds(&[]);
    }

    pub fn fold_all(&mut self, fold: bool) {
        self.refresh_folds();
        self.folds.clear();
        if fold {
            let text = self.editor.buf.text();
            for r in fold_ranges(self.editor.buf.doc(), &text) {
                self.folds.fold(r.header);
            }
        }
        self.hidden_dirty = true;
        self.refresh_folds();
        self.leave_folds(&[]);
    }

    /// Move cursors out of folded text. `before` holds the heads before the motion that got
    /// them there: a cursor that moved forward leaves through the end of the fold, any other
    /// goes to the end of the fold's visible line.
    pub fn leave_folds(&mut self, before: &[usize]) {
        let buf = &self.editor.buf;
        if self.hidden.1.is_empty() || self.hidden.0 != buf.version {
            return;
        }
        let len = buf.len();
        let mut moved = false;
        let sels: Vec<Sel> = buf
            .sels
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let Some(&(hs, he)) = self.hidden.1.iter().find(|(hs, he)| *hs < s.head && s.head <= *he)
                else {
                    return *s;
                };
                moved = true;
                let forward = before.get(i).is_some_and(|&b| s.head > b) && he < len;
                let to = if forward { he + 1 } else { hs };
                if s.is_empty() {
                    Sel::cursor(to)
                } else {
                    Sel::new(s.anchor, to)
                }
            })
            .collect();
        if moved {
            self.editor.buf.set_sels(sels);
        }
    }

    // ---- vim ----

    /// Feed a key to vim. Returns true if vim consumed it.
    pub fn vim_key(&mut self, key: &str, now: u64) -> bool {
        let Some(vim) = &mut self.vim else { return false };
        let v = self.editor.buf.version;
        let consumed = vim.key(&mut self.editor, key, now);
        if self.editor.buf.version != v {
            self.last_edit_ms = now;
        }
        consumed
    }

    /// Whether typed text goes to vim as commands instead of into the note.
    pub fn vim_commands(&self) -> bool {
        self.mode != ViewMode::Reading && self.vim.as_ref().is_some_and(|v| v.mode != VimMode::Insert)
    }

    pub fn vim_label(&self) -> Option<&'static str> {
        Some(match self.vim.as_ref()?.mode {
            VimMode::Normal => "NORMAL",
            VimMode::Insert => "INSERT",
            VimMode::Visual => "VISUAL",
            VimMode::VisualLine => "V-LINE",
        })
    }
}

impl App {
    /// Editor commands implemented here. Returns false if `command` is not one of them.
    pub fn run_editing(&mut self, command: &str, now: u64) -> bool {
        let Some(t) = self.tab_mut() else { return false };
        if let Some(header) = command.strip_prefix("fold:").and_then(|h| h.parse().ok()) {
            t.toggle_fold(header);
            return true;
        }
        if let Some(op) = table_op(command) {
            if t.mode != ViewMode::Reading {
                t.table_op(op, now);
            }
            return true;
        }
        match command {
            "editor:toggle-fold" => t.toggle_fold_at_cursor(),
            "editor:fold-all" => t.fold_all(true),
            "editor:unfold-all" => t.fold_all(false),
            "editor:insert-table" if t.mode != ViewMode::Reading => t.insert_table(now),
            "editor:toggle-vim" => {
                let on = self.tabs.iter().any(|t| t.vim.is_some());
                for t in &mut self.tabs {
                    t.vim = (!on).then(Default::default);
                }
                self.vim = !on;
            }
            _ => return false,
        }
        true
    }
}
