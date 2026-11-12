//! Obsidian search query language.
//!
//! `a b` (and), `a OR b`, `-a`, `( … )`, `"exact phrase"`, `/regex/`, `file:`, `path:`,
//! `content:`, `tag:`, `line:`, `block:`, `section:`, `task:`, `task-todo:`, `task-done:`,
//! `match-case:`, `ignore-case:`, `[property]`, `[property:value]`, `[property:>5]`.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    File,
    Path,
    Content,
    Tag,
    Line,
    Block,
    Section,
    Task,
    TaskTodo,
    TaskDone,
    MatchCase,
    IgnoreCase,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, PartialEq, Debug)]
pub enum PropValue {
    /// `[key]`: the property exists.
    Any,
    /// `[key:null]`: the property exists and is empty.
    Null,
    Compare(Cmp, f64),
    Expr(Box<Expr>),
}

#[derive(Clone, PartialEq, Debug)]
pub enum Expr {
    And(Vec<Expr>),
    Or(Vec<Expr>),
    Not(Box<Expr>),
    /// A word or quoted phrase.
    Term(String),
    Regex(String),
    Op(Op, Box<Expr>),
    Property(String, PropValue),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ParseError {
    pub offset: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (at {})", self.message, self.offset)
    }
}
impl std::error::Error for ParseError {}

pub fn parse(q: &str) -> Result<Expr, ParseError> {
    let mut p = P { s: q, i: 0, depth: 0 };
    let e = p.or()?;
    p.ws();
    if p.i < q.len() {
        return Err(p.err("unexpected `)`"));
    }
    Ok(e)
}

struct P<'a> {
    s: &'a str,
    i: usize,
    depth: u32,
}

const OPS: &[(&str, Op)] = &[
    ("file:", Op::File),
    ("path:", Op::Path),
    ("content:", Op::Content),
    ("tag:", Op::Tag),
    ("line:", Op::Line),
    ("block:", Op::Block),
    ("section:", Op::Section),
    ("task-todo:", Op::TaskTodo),
    ("task-done:", Op::TaskDone),
    ("task:", Op::Task),
    ("match-case:", Op::MatchCase),
    ("ignore-case:", Op::IgnoreCase),
];

impl<'a> P<'a> {
    fn err(&self, m: &str) -> ParseError {
        ParseError { offset: self.i, message: m.to_string() }
    }
    fn rest(&self) -> &'a str {
        &self.s[self.i..]
    }
    fn ws(&mut self) {
        self.i += self.rest().len() - self.rest().trim_start().len();
    }

    fn or(&mut self) -> Result<Expr, ParseError> {
        self.depth += 1;
        if self.depth > 64 {
            return Err(self.err("query nested too deeply"));
        }
        let mut alts = vec![self.and()?];
        loop {
            self.ws();
            let r = self.rest();
            if r.starts_with("OR") && r[2..].chars().next().is_none_or(|c| c.is_whitespace() || c == '(') {
                self.i += 2;
                alts.push(self.and()?);
            } else {
                break;
            }
        }
        self.depth -= 1;
        Ok(if alts.len() == 1 { alts.remove(0) } else { Expr::Or(alts) })
    }

    fn and(&mut self) -> Result<Expr, ParseError> {
        let mut parts = Vec::new();
        loop {
            self.ws();
            let r = self.rest();
            if r.is_empty() || r.starts_with(')') {
                break;
            }
            if r.starts_with("OR")
                && r[2..].chars().next().is_none_or(|c| c.is_whitespace() || c == '(')
                && !parts.is_empty()
            {
                break;
            }
            parts.push(self.unary()?);
        }
        Ok(if parts.len() == 1 { parts.remove(0) } else { Expr::And(parts) })
    }

    fn unary(&mut self) -> Result<Expr, ParseError> {
        if self.rest().starts_with('-')
            && self.rest().len() > 1
            && !self.rest()[1..].starts_with(char::is_whitespace)
        {
            self.i += 1;
            return Ok(Expr::Not(Box::new(self.atom()?)));
        }
        self.atom()
    }

    fn atom(&mut self) -> Result<Expr, ParseError> {
        let r = self.rest();
        if r.starts_with('(') {
            self.i += 1;
            let e = self.or()?;
            self.ws();
            if !self.rest().starts_with(')') {
                return Err(self.err("missing `)`"));
            }
            self.i += 1;
            return Ok(e);
        }
        if let Some(body) = r.strip_prefix('"') {
            let mut out = String::new();
            let mut chars = body.char_indices();
            while let Some((k, c)) = chars.next() {
                match c {
                    '\\' => {
                        if let Some((_, n)) = chars.next() {
                            out.push(n);
                        }
                    }
                    '"' => {
                        self.i += 1 + k + 1;
                        return Ok(Expr::Term(out));
                    }
                    c => out.push(c),
                }
            }
            // Unterminated quote: take the rest as the phrase, as Obsidian does while typing.
            self.i = self.s.len();
            return Ok(Expr::Term(out));
        }
        if r.starts_with('/') {
            let b = r.as_bytes();
            let mut k = 1;
            while k < b.len() && b[k] != b'/' {
                if b[k] == b'\\' {
                    k += 1;
                }
                k += 1;
            }
            if k < b.len() && k > 1 {
                let pat = r[1..k].to_string();
                self.i += k + 1;
                regex::Regex::new(&pat).map_err(|e| self.err(&format!("invalid regex: {e}")))?;
                return Ok(Expr::Regex(pat));
            }
        }
        if r.starts_with('[') {
            if let Some(close) = matching_bracket(r) {
                let inner = &r[1..close];
                let start = self.i;
                self.i += close + 1;
                let (key, value) = match inner.split_once(':') {
                    Some((k, v)) => (k.trim(), Some(v.trim())),
                    None => (inner.trim(), None),
                };
                if !key.is_empty() {
                    let value = match value {
                        None | Some("") => PropValue::Any,
                        Some("null") => PropValue::Null,
                        Some(v) => match compare(v) {
                            Some((c, n)) => PropValue::Compare(c, n),
                            None => PropValue::Expr(Box::new(parse(v).map_err(|e| ParseError {
                                offset: start + 1 + e.offset,
                                message: e.message,
                            })?)),
                        },
                    };
                    return Ok(Expr::Property(key.to_string(), value));
                }
                self.i = start;
            }
        }
        for (name, op) in OPS {
            if r.len() >= name.len() && r[..name.len()].eq_ignore_ascii_case(name) {
                self.i += name.len();
                if self.rest().is_empty() || self.rest().starts_with(char::is_whitespace) {
                    // `tag:` with nothing after it matches everything, so typing stays responsive.
                    return Ok(Expr::Op(*op, Box::new(Expr::Term(String::new()))));
                }
                return Ok(Expr::Op(*op, Box::new(self.atom()?)));
            }
        }
        // Bare word: up to whitespace or a closing parenthesis.
        let end = r.find(|c: char| c.is_whitespace() || c == ')').unwrap_or(r.len());
        if end == 0 {
            return Err(self.err("unexpected character"));
        }
        self.i += end;
        Ok(Expr::Term(r[..end].to_string()))
    }
}

fn matching_bracket(r: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in r.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn compare(v: &str) -> Option<(Cmp, f64)> {
    let (c, rest) = [("<=", Cmp::Le), (">=", Cmp::Ge), ("<", Cmp::Lt), (">", Cmp::Gt)]
        .into_iter()
        .find_map(|(p, c)| v.strip_prefix(p).map(|r| (c, r)))?;
    rest.trim().parse().ok().map(|n| (c, n))
}

#[cfg(test)]
mod tests {
    use super::Expr::{self, *};
    use super::Op as O;
    use super::{parse, Cmp, PropValue};

    fn t(s: &str) -> Expr {
        Term(s.into())
    }
    fn b(e: Expr) -> Box<Expr> {
        Box::new(e)
    }

    #[test]
    fn boolean_structure() {
        assert_eq!(parse("a b").unwrap(), And(vec![t("a"), t("b")]));
        assert_eq!(parse("a OR b c").unwrap(), Or(vec![t("a"), And(vec![t("b"), t("c")])]));
        assert_eq!(parse("(a OR b) -c").unwrap(), And(vec![Or(vec![t("a"), t("b")]), Not(b(t("c")))]));
        assert_eq!(parse("\"exact phrase\" x").unwrap(), And(vec![t("exact phrase"), t("x")]));
        assert_eq!(parse("ORANGE").unwrap(), t("ORANGE"));
        assert_eq!(parse("a - b").unwrap(), And(vec![t("a"), t("-"), t("b")]));
        assert_eq!(parse("").unwrap(), And(vec![]));
    }

    #[test]
    fn operators() {
        assert_eq!(parse("file:.jpg").unwrap(), Op(O::File, b(t(".jpg"))));
        assert_eq!(parse("path:\"Daily notes/2022\"").unwrap(), Op(O::Path, b(t("Daily notes/2022"))));
        assert_eq!(
            parse("tag:#work -tag:#done").unwrap(),
            And(vec![Op(O::Tag, b(t("#work"))), Not(b(Op(O::Tag, b(t("#done")))))])
        );
        assert_eq!(parse("line:(mix flour)").unwrap(), Op(O::Line, b(And(vec![t("mix"), t("flour")]))));
        assert_eq!(parse("task-todo:call").unwrap(), Op(O::TaskTodo, b(t("call"))));
        assert_eq!(parse("match-case:HappyCat").unwrap(), Op(O::MatchCase, b(t("HappyCat"))));
        assert_eq!(parse("content:/\\d{4}/").unwrap(), Op(O::Content, b(Regex("\\d{4}".into()))));
        assert_eq!(parse("task:").unwrap(), Op(O::Task, b(t(""))));
        for op in ["block:", "section:", "task-done:", "ignore-case:", "content:"] {
            assert!(matches!(parse(&format!("{op}x")).unwrap(), Op(..)), "{op}");
        }
    }

    #[test]
    fn properties() {
        assert_eq!(parse("[aliases]").unwrap(), Property("aliases".into(), PropValue::Any));
        assert_eq!(parse("[status:null]").unwrap(), Property("status".into(), PropValue::Null));
        assert_eq!(
            parse("[duration:<5]").unwrap(),
            Property("duration".into(), PropValue::Compare(Cmp::Lt, 5.0))
        );
        assert_eq!(
            parse("[status:Draft OR Published]").unwrap(),
            Property("status".into(), PropValue::Expr(b(Or(vec![t("Draft"), t("Published")]))))
        );
    }

    #[test]
    fn errors() {
        assert!(parse("(a b").is_err());
        assert!(parse("a) b").is_err());
        assert!(parse("/(unclosed/").is_err());
        assert_eq!(parse("a/b").unwrap(), t("a/b"));
    }
}
