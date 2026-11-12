//! Table editing: parse a Markdown table into a grid, edit it, write it back aligned.

use md_engine::Align;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Table {
    pub aligns: Vec<Align>,
    /// Row 0 is the header.
    pub rows: Vec<Vec<String>>,
}

fn split_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cur.push_str("\\|");
                chars.next();
            }
            '|' => cells.push(std::mem::take(&mut cur).trim().to_string()),
            c => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        cells.push(cur.trim().to_string());
    }
    cells
}

impl Table {
    pub fn new(cols: usize, rows: usize) -> Table {
        Table {
            aligns: vec![Align::None; cols.max(1)],
            rows: vec![vec![String::new(); cols.max(1)]; rows.max(1)],
        }
    }

    /// Parse the source of a table (header, delimiter row, body rows).
    pub fn parse(src: &str) -> Option<Table> {
        let mut lines = src.lines().filter(|l| !l.trim().is_empty());
        let header = split_row(lines.next()?);
        let aligns: Vec<Align> = split_row(lines.next()?)
            .iter()
            .map(|c| {
                let (l, r) = (c.starts_with(':'), c.ends_with(':') && c.len() > 1);
                let dashes = c.trim_matches(':');
                if dashes.is_empty() || !dashes.bytes().all(|b| b == b'-') {
                    return None;
                }
                Some(match (l, r) {
                    (true, true) => Align::Center,
                    (true, false) => Align::Left,
                    (false, true) => Align::Right,
                    _ => Align::None,
                })
            })
            .collect::<Option<_>>()?;
        if aligns.is_empty() {
            return None;
        }
        let mut rows = vec![header];
        rows.extend(lines.map(split_row));
        for r in &mut rows {
            r.resize(aligns.len(), String::new());
        }
        Some(Table { aligns, rows })
    }

    pub fn cols(&self) -> usize {
        self.aligns.len()
    }

    pub fn insert_row(&mut self, at: usize) {
        let at = at.clamp(1, self.rows.len());
        self.rows.insert(at, vec![String::new(); self.cols()]);
    }
    pub fn delete_row(&mut self, at: usize) {
        if at > 0 && at < self.rows.len() {
            self.rows.remove(at);
        }
    }
    pub fn insert_col(&mut self, at: usize) {
        let at = at.min(self.cols());
        self.aligns.insert(at, Align::None);
        for r in &mut self.rows {
            r.insert(at, String::new());
        }
    }
    pub fn delete_col(&mut self, at: usize) {
        if self.cols() > 1 && at < self.cols() {
            self.aligns.remove(at);
            for r in &mut self.rows {
                r.remove(at);
            }
        }
    }
    pub fn move_col(&mut self, from: usize, to: usize) {
        if from < self.cols() && to < self.cols() {
            let a = self.aligns.remove(from);
            self.aligns.insert(to, a);
            for r in &mut self.rows {
                let c = r.remove(from);
                r.insert(to, c);
            }
        }
    }
    pub fn move_row(&mut self, from: usize, to: usize) {
        if from > 0 && to > 0 && from < self.rows.len() && to < self.rows.len() {
            let r = self.rows.remove(from);
            self.rows.insert(to, r);
        }
    }
    /// Sort body rows by a column; numeric when every cell parses as a number.
    pub fn sort_by(&mut self, col: usize, descending: bool) {
        if col >= self.cols() || self.rows.len() < 3 {
            return;
        }
        let body = &mut self.rows[1..];
        let numeric = body.iter().all(|r| r[col].trim().parse::<f64>().is_ok());
        body.sort_by(|a, b| {
            if numeric {
                let (x, y) = (
                    a[col].trim().parse::<f64>().unwrap_or(0.0),
                    b[col].trim().parse::<f64>().unwrap_or(0.0),
                );
                x.total_cmp(&y)
            } else {
                a[col].to_lowercase().cmp(&b[col].to_lowercase())
            }
        });
        if descending {
            body.reverse();
        }
    }

    /// Tab / Shift+Tab navigation. Moving past the last cell appends a row.
    pub fn next_cell(&mut self, row: usize, col: usize, forward: bool) -> (usize, usize) {
        if forward {
            if col + 1 < self.cols() {
                (row, col + 1)
            } else {
                if row + 1 >= self.rows.len() {
                    self.insert_row(self.rows.len());
                }
                (row + 1, 0)
            }
        } else if col > 0 {
            (row, col - 1)
        } else if row > 0 {
            (row - 1, self.cols() - 1)
        } else {
            (0, 0)
        }
    }

    /// Markdown source with padded, aligned columns. Width is measured in chars.
    pub fn to_markdown(&self) -> String {
        let width = |s: &str| s.chars().count();
        let widths: Vec<usize> = (0..self.cols())
            .map(|c| self.rows.iter().map(|r| width(&r[c])).max().unwrap_or(0).max(3))
            .collect();
        let mut out = String::new();
        let line = |out: &mut String, cells: &[String]| {
            out.push('|');
            for (c, cell) in cells.iter().enumerate() {
                let pad = widths[c] - width(cell);
                let (l, r) = match self.aligns[c] {
                    Align::Right => (pad, 0),
                    Align::Center => (pad / 2, pad - pad / 2),
                    _ => (0, pad),
                };
                out.push(' ');
                out.push_str(&" ".repeat(l));
                out.push_str(cell);
                out.push_str(&" ".repeat(r));
                out.push_str(" |");
            }
            out.push('\n');
        };
        line(&mut out, &self.rows[0]);
        out.push('|');
        for (c, a) in self.aligns.iter().enumerate() {
            let w = widths[c];
            let cell = match a {
                Align::Left => format!(":{}", "-".repeat(w - 1)),
                Align::Right => format!("{}:", "-".repeat(w - 1)),
                Align::Center => format!(":{}:", "-".repeat(w - 2)),
                Align::None => "-".repeat(w),
            };
            out.push(' ');
            out.push_str(&cell);
            out.push_str(" |");
        }
        out.push('\n');
        for r in &self.rows[1..] {
            line(&mut out, r);
        }
        out
    }

    /// Byte offset of a cell's content start within `to_markdown()` output.
    pub fn cell_offset(&self, row: usize, col: usize) -> usize {
        let md = self.to_markdown();
        let line_idx = if row == 0 { 0 } else { row + 1 };
        let line_start: usize = md.lines().take(line_idx).map(|l| l.len() + 1).sum();
        let line = md.lines().nth(line_idx).unwrap_or("");
        let mut pipes = 0;
        for (i, c) in line.char_indices() {
            if c == '|' && !line[..i].ends_with('\\') {
                if pipes == col {
                    return line_start + i + 2;
                }
                pipes += 1;
            }
        }
        line_start
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_format_round_trip() {
        let t = Table::parse("|a|b|c\n|:-|:-:|--:|\n|1|two|3|\n| x \\| y | | long cell |").unwrap();
        assert_eq!(t.aligns, vec![Align::Left, Align::Center, Align::Right]);
        assert_eq!(t.rows[2], vec!["x \\| y", "", "long cell"]);
        let md = t.to_markdown();
        assert_eq!(md, "| a      |  b  |         c |\n| :----- | :-: | --------: |\n| 1      | two |         3 |\n| x \\| y |     | long cell |\n");
        assert_eq!(Table::parse(&md).unwrap(), t);
        assert!(md_engine::html::render(&md_engine::parse(&md), &md).contains("<table>"));
        assert!(Table::parse("not a table\nat all").is_none());
    }

    #[test]
    fn structural_edits() {
        let mut t = Table::parse("| n | name |\n|---|---|\n| 10 | b |\n| 9 | a |\n").unwrap();
        t.sort_by(0, false);
        assert_eq!(t.rows[1][0], "9", "numeric sort");
        t.sort_by(1, true);
        assert_eq!(t.rows[1][1], "b");
        t.insert_col(1);
        t.rows[0][1] = "new".into();
        t.move_col(1, 2);
        assert_eq!(t.rows[0], vec!["n", "name", "new"]);
        t.delete_col(0);
        t.insert_row(1);
        t.delete_row(0);
        assert_eq!((t.cols(), t.rows.len()), (2, 4), "the header row cannot be deleted");
        t.move_row(1, 3);
        assert_eq!(t.rows[3], vec!["", ""]);
    }

    #[test]
    fn cell_navigation() {
        let mut t = Table::new(2, 2);
        assert_eq!(t.next_cell(0, 0, true), (0, 1));
        assert_eq!(t.next_cell(0, 1, true), (1, 0));
        assert_eq!(t.next_cell(1, 1, true), (2, 0));
        assert_eq!(t.rows.len(), 3, "tabbing past the end adds a row");
        assert_eq!(t.next_cell(1, 0, false), (0, 1));
        t.rows[2][1] = "here".into();
        let md = t.to_markdown();
        assert!(md[t.cell_offset(2, 1)..].starts_with("here"));
    }
}
