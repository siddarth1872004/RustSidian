//! Front matter: the YAML subset Obsidian properties use.
//!
//! Supported: block maps and lists, flow lists/maps, plain/single/double quoted scalars, `|` and
//! `>` block scalars, comments. Not supported: anchors, aliases, tags, multi-document streams,
//! complex keys.

use std::fmt::Write;

#[derive(Clone, PartialEq, Debug)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    List(Vec<Value>),
    Map(Vec<(String, Value)>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }
    /// Scalars as text; lists flattened. Used for tags, aliases and cssclasses, which Obsidian
    /// accepts as either a list or a single (possibly comma separated) string.
    pub fn as_string_list(&self) -> Vec<String> {
        match self {
            Value::Null => Vec::new(),
            Value::List(items) => items.iter().flat_map(|v| v.as_string_list()).collect(),
            Value::String(s) => {
                s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
            }
            Value::Bool(b) => vec![b.to_string()],
            Value::Number(n) => vec![fmt_number(*n)],
            Value::Map(_) => Vec::new(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Error {
    /// 1-based line within the front matter.
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "front matter line {}: {}", self.line, self.message)
    }
}
impl std::error::Error for Error {}

pub type Properties = Vec<(String, Value)>;

struct L<'a> {
    no: usize,
    indent: usize,
    text: &'a str,
}

pub fn parse(src: &str) -> Result<Properties, Error> {
    let lines: Vec<L> = src
        .lines()
        .enumerate()
        .map(|(i, l)| {
            let t = l.trim_start_matches(' ');
            L { no: i + 1, indent: l.len() - t.len(), text: t.trim_end() }
        })
        .collect();
    let mut p = P { lines, i: 0, depth: 0 };
    p.skip();
    if p.i >= p.lines.len() {
        return Ok(Vec::new());
    }
    let indent = p.lines[p.i].indent;
    match p.block(indent)? {
        Value::Map(m) => {
            p.skip();
            if p.i < p.lines.len() {
                return Err(p.err("unexpected content"));
            }
            Ok(m)
        }
        _ => Err(Error { line: 1, message: "front matter must be a mapping".into() }),
    }
}

struct P<'a> {
    lines: Vec<L<'a>>,
    i: usize,
    depth: u32,
}

impl<'a> P<'a> {
    fn err(&self, m: &str) -> Error {
        Error { line: self.lines.get(self.i).map_or(self.lines.len(), |l| l.no), message: m.into() }
    }
    fn skip(&mut self) {
        while self.i < self.lines.len()
            && (self.lines[self.i].text.is_empty() || self.lines[self.i].text.starts_with('#'))
        {
            self.i += 1;
        }
    }

    fn block(&mut self, indent: usize) -> Result<Value, Error> {
        self.depth += 1;
        if self.depth > 64 {
            return Err(self.err("nesting too deep"));
        }
        let t = self.lines[self.i].text;
        let v = if t == "-" || t.starts_with("- ") { self.list(indent) } else { self.map(indent) };
        self.depth -= 1;
        v
    }

    fn list(&mut self, indent: usize) -> Result<Value, Error> {
        let mut items = Vec::new();
        loop {
            self.skip();
            let Some(l) = self.lines.get(self.i) else { break };
            if l.indent != indent || !(l.text == "-" || l.text.starts_with("- ")) {
                if l.indent > indent {
                    return Err(self.err("bad indentation"));
                }
                break;
            }
            let rest = l.text[1..].trim_start();
            let inner_indent = l.indent + (l.text.len() - rest.len());
            if rest.is_empty() {
                self.i += 1;
                items.push(self.nested(indent)?);
            } else if split_key(rest).is_some() && !rest.starts_with(['[', '{', '"', '\'']) {
                // `- key: value` starts an inline map; rewrite the line as a map line.
                self.lines[self.i].indent = inner_indent;
                self.lines[self.i].text = rest;
                items.push(self.map(inner_indent)?);
            } else {
                self.i += 1;
                items.push(self.scalar_or_flow(rest, indent)?);
            }
        }
        Ok(Value::List(items))
    }

    /// Value on the lines after `key:` or `-`, indented deeper than `indent`.
    fn nested(&mut self, indent: usize) -> Result<Value, Error> {
        self.skip();
        match self.lines.get(self.i) {
            Some(l) if l.indent > indent => self.block(l.indent),
            // YAML allows a list at the same indentation as its key.
            Some(l) if l.indent == indent && (l.text == "-" || l.text.starts_with("- ")) => self.list(indent),
            _ => Ok(Value::Null),
        }
    }

    fn map(&mut self, indent: usize) -> Result<Value, Error> {
        let mut out: Vec<(String, Value)> = Vec::new();
        loop {
            self.skip();
            let Some(l) = self.lines.get(self.i) else { break };
            if l.indent < indent {
                break;
            }
            if l.indent > indent {
                return Err(self.err("bad indentation"));
            }
            let text = l.text;
            let Some((key, rest)) = split_key(text) else {
                if text == "-" || text.starts_with("- ") {
                    break;
                }
                return Err(self.err("expected `key: value`"));
            };
            self.i += 1;
            let value = if rest.is_empty() {
                self.nested(indent)?
            } else if let Some(style) =
                rest.strip_prefix(['|', '>']).filter(|r| r.chars().all(|c| "+-0123456789".contains(c)))
            {
                self.block_scalar(indent, rest.starts_with('|'), style)
            } else {
                self.scalar_or_flow(rest, indent)?
            };
            if let Some(slot) = out.iter_mut().find(|(k, _)| *k == key) {
                slot.1 = value;
            } else {
                out.push((key, value));
            }
        }
        Ok(Value::Map(out))
    }

    fn block_scalar(&mut self, indent: usize, literal: bool, style: &str) -> Value {
        let mut parts: Vec<&str> = Vec::new();
        while let Some(l) = self.lines.get(self.i) {
            if l.text.is_empty() {
                parts.push("");
            } else if l.indent > indent {
                // Indentation beyond the first content line is not preserved.
                parts.push(l.text);
            } else {
                break;
            }
            self.i += 1;
        }
        while parts.last() == Some(&"") {
            parts.pop();
        }
        let mut s = if literal {
            parts.join("\n")
        } else {
            let mut s = String::new();
            for (i, p) in parts.iter().enumerate() {
                if i > 0 {
                    s.push(if p.is_empty() || parts[i - 1].is_empty() { '\n' } else { ' ' });
                }
                s.push_str(p);
            }
            s
        };
        if !style.contains('-') && !s.is_empty() {
            s.push('\n');
        }
        Value::String(s)
    }

    fn scalar_or_flow(&mut self, rest: &'a str, indent: usize) -> Result<Value, Error> {
        if rest.starts_with('[') || rest.starts_with('{') {
            // A flow collection may continue on following, deeper-indented lines.
            let mut buf = rest.to_string();
            while !flow_balanced(&buf) {
                match self.lines.get(self.i) {
                    Some(l) if l.indent > indent || l.text.starts_with([']', '}']) => {
                        buf.push(' ');
                        buf.push_str(l.text);
                        self.i += 1;
                    }
                    _ => break,
                }
            }
            let mut f = Flow { b: buf.as_bytes(), s: &buf, i: 0, depth: 0 };
            return match f.value() {
                Some(v) if f.rest_is_empty() => Ok(v),
                // Obsidian treats `[[link]]`-looking plain text as a string.
                _ => Ok(scalar(rest)),
            };
        }
        Ok(scalar(rest))
    }
}

fn flow_balanced(s: &str) -> bool {
    let (mut depth, mut quote) = (0i32, None);
    let mut prev = ' ';
    for c in s.chars() {
        match quote {
            Some(q) => {
                if c == q && !(q == '"' && prev == '\\') {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '[' | '{' => depth += 1,
                ']' | '}' => depth -= 1,
                _ => {}
            },
        }
        prev = c;
    }
    depth <= 0
}

/// Splits `key: rest`. The key may be quoted.
fn split_key(text: &str) -> Option<(String, &str)> {
    let b = text.as_bytes();
    if let Some(&q) = b.first().filter(|c| matches!(c, b'"' | b'\'')) {
        let close = text[1..].find(q as char)? + 1;
        let after = &text[close + 1..];
        let rest = after.strip_prefix(':')?;
        if !rest.is_empty() && !rest.starts_with(' ') {
            return None;
        }
        return Some((text[1..close].to_string(), rest.trim_start()));
    }
    let mut from = 0;
    loop {
        let p = text[from..].find(':')? + from;
        let after = &text[p + 1..];
        if after.is_empty() || after.starts_with(' ') {
            let key = text[..p].trim_end();
            if key.is_empty() {
                return None;
            }
            return Some((key.to_string(), after.trim_start()));
        }
        from = p + 1;
    }
}

fn strip_comment(s: &str) -> &str {
    match s.find(" #") {
        Some(p) => s[..p].trim_end(),
        None => s,
    }
}

fn unquote_double(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('u') => {
                let hex: String = it.by_ref().take(4).collect();
                out.push(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32).unwrap_or('\u{fffd}'));
            }
            Some(o) => out.push(o),
            None => out.push('\\'),
        }
    }
    out
}

fn scalar(raw: &str) -> Value {
    let raw = raw.trim();
    if let Some(q) = raw.chars().next().filter(|c| matches!(c, '"' | '\'')) {
        // Find the closing quote; anything after it must be a comment.
        let b = raw.as_bytes();
        let mut k = 1;
        while k < b.len() {
            if q == '"' && b[k] == b'\\' {
                k += 1;
            } else if b[k] == q as u8 {
                if q == '\'' && b.get(k + 1) == Some(&b'\'') {
                    k += 1;
                } else {
                    break;
                }
            }
            k += 1;
        }
        if k < b.len() {
            let tail = raw[k + 1..].trim_start();
            if tail.is_empty() || tail.starts_with('#') {
                let inner = &raw[1..k];
                return Value::String(if q == '"' {
                    unquote_double(inner)
                } else {
                    inner.replace("''", "'")
                });
            }
        }
    }
    let s = if raw.starts_with('#') { "" } else { strip_comment(raw) };
    plain(s)
}

fn plain(s: &str) -> Value {
    match s {
        "" | "~" | "null" | "Null" | "NULL" => return Value::Null,
        "true" | "True" | "TRUE" => return Value::Bool(true),
        "false" | "False" | "FALSE" => return Value::Bool(false),
        _ => {}
    }
    let numeric = s.bytes().all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'-' | b'+' | b'e' | b'E'))
        && s.bytes().any(|c| c.is_ascii_digit())
        // Leading zeros (ids, zip codes) and dates stay strings.
        && !(s.len() > 1 && s.starts_with('0') && !s.starts_with("0."))
        && !s[1..].contains('-');
    if numeric {
        if let Ok(n) = s.parse::<f64>() {
            if n.is_finite() {
                return Value::Number(n);
            }
        }
    }
    Value::String(s.to_string())
}

struct Flow<'a> {
    b: &'a [u8],
    s: &'a str,
    i: usize,
    depth: u32,
}

impl Flow<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn rest_is_empty(&mut self) -> bool {
        self.ws();
        self.i >= self.b.len() || self.b[self.i] == b'#'
    }
    fn value(&mut self) -> Option<Value> {
        self.ws();
        self.depth += 1;
        if self.depth > 64 {
            return None;
        }
        let v = match *self.b.get(self.i)? {
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.ws();
                    if *self.b.get(self.i)? == b']' {
                        self.i += 1;
                        break;
                    }
                    items.push(self.value()?);
                    self.ws();
                    match *self.b.get(self.i)? {
                        b',' => self.i += 1,
                        b']' => {}
                        _ => return None,
                    }
                }
                Value::List(items)
            }
            b'{' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.ws();
                    if *self.b.get(self.i)? == b'}' {
                        self.i += 1;
                        break;
                    }
                    let Value::String(k) = self.atom(true)? else { return None };
                    self.ws();
                    if *self.b.get(self.i)? != b':' {
                        return None;
                    }
                    self.i += 1;
                    items.push((k, self.value()?));
                    self.ws();
                    match *self.b.get(self.i)? {
                        b',' => self.i += 1,
                        b'}' => {}
                        _ => return None,
                    }
                }
                Value::Map(items)
            }
            _ => self.atom(false)?,
        };
        self.depth -= 1;
        Some(v)
    }
    fn atom(&mut self, key: bool) -> Option<Value> {
        let q = *self.b.get(self.i)?;
        if q == b'"' || q == b'\'' {
            let start = self.i + 1;
            let mut k = start;
            while k < self.b.len() && self.b[k] != q {
                if q == b'"' && self.b[k] == b'\\' {
                    k += 1;
                }
                k += 1;
            }
            if k >= self.b.len() {
                return None;
            }
            self.i = k + 1;
            let inner = &self.s[start..k];
            return Some(Value::String(if q == b'"' {
                unquote_double(inner)
            } else {
                inner.replace("''", "'")
            }));
        }
        let start = self.i;
        while self.i < self.b.len()
            && !matches!(self.b[self.i], b',' | b']' | b'}' | b'[' | b'{')
            && !(key && self.b[self.i] == b':')
        {
            self.i += 1;
        }
        let s = self.s[start..self.i].trim();
        if s.is_empty() {
            return None;
        }
        Some(if key { Value::String(s.to_string()) } else { plain(s) })
    }
}

pub fn fmt_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

fn needs_quotes(s: &str) -> bool {
    s.is_empty()
        || s != s.trim()
        || !matches!(plain(s), Value::String(_))
        || s.starts_with([
            '-', '?', ':', ',', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '\'', '"', '%', '@', '`',
        ])
        || s.contains(": ")
        || s.contains(" #")
        || s.ends_with(':')
        || s.contains(['\n', '\t', '\r'])
}

fn write_scalar(out: &mut String, s: &str) {
    if needs_quotes(s) {
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                '\r' => out.push_str("\\r"),
                c => out.push(c),
            }
        }
        out.push('"');
    } else {
        out.push_str(s);
    }
}

fn write_value(out: &mut String, v: &Value, indent: usize) {
    match v {
        Value::Null => out.push('\n'),
        Value::Bool(b) => _ = writeln!(out, " {b}"),
        Value::Number(n) => _ = writeln!(out, " {}", fmt_number(*n)),
        Value::String(s) => {
            out.push(' ');
            write_scalar(out, s);
            out.push('\n');
        }
        Value::List(items) if items.is_empty() => out.push_str(" []\n"),
        Value::Map(items) if items.is_empty() => out.push_str(" {}\n"),
        Value::List(items) => {
            out.push('\n');
            for it in items {
                let _ = write!(out, "{:indent$}  -", "");
                match it {
                    Value::Map(m) if !m.is_empty() => {
                        for (i, (k, v)) in m.iter().enumerate() {
                            if i == 0 {
                                out.push(' ');
                            } else {
                                let _ = write!(out, "{:w$}", "", w = indent + 4);
                            }
                            write_scalar(out, k);
                            out.push(':');
                            write_value(out, v, indent + 4);
                        }
                    }
                    other => write_value(out, other, indent + 2),
                }
            }
        }
        Value::Map(items) => {
            out.push('\n');
            write_map(out, items, indent + 2);
        }
    }
}

fn write_map(out: &mut String, items: &[(String, Value)], indent: usize) {
    for (k, v) in items {
        let _ = write!(out, "{:indent$}", "");
        write_scalar(out, k);
        out.push(':');
        write_value(out, v, indent);
    }
}

/// Serialize properties (without the `---` fences).
pub fn to_string(props: &[(String, Value)]) -> String {
    let mut out = String::new();
    write_map(&mut out, props, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> Value {
        Value::String(x.into())
    }

    #[test]
    fn scalars_and_lists() {
        let p = parse(
            "title: Hello World\ncount: 3\nratio: 1.5\ndone: true\nempty:\ndate: 2024-01-05\nzip: 01234\ntags: [a, b/c, \"d e\"]\naliases:\n  - One\n  - \"Two: 2\"\nlink: \"[[Note]]\" # comment\n",
        )
        .unwrap();
        assert_eq!(p[0], ("title".into(), s("Hello World")));
        assert_eq!(p[1].1, Value::Number(3.0));
        assert_eq!(p[2].1, Value::Number(1.5));
        assert_eq!(p[3].1, Value::Bool(true));
        assert_eq!(p[4].1, Value::Null);
        assert_eq!(p[5].1, s("2024-01-05"));
        assert_eq!(p[6].1, s("01234"));
        assert_eq!(p[7].1, Value::List(vec![s("a"), s("b/c"), s("d e")]));
        assert_eq!(p[8].1, Value::List(vec![s("One"), s("Two: 2")]));
        assert_eq!(p[9].1, s("[[Note]]"));
    }

    #[test]
    fn nested_and_block_scalars() {
        let p = parse("meta:\n  author: Ann\n  ids:\n  - 1\n  - 2\nnote: |\n  line one\n  line two\nfold: >-\n  a\n  b\nitems:\n  - name: x\n    n: 1\n  - name: y\n").unwrap();
        assert_eq!(
            p[0].1,
            Value::Map(vec![
                ("author".into(), s("Ann")),
                ("ids".into(), Value::List(vec![Value::Number(1.0), Value::Number(2.0)]))
            ])
        );
        assert_eq!(p[1].1, s("line one\nline two\n"));
        assert_eq!(p[2].1, s("a b"));
        assert_eq!(
            p[3].1,
            Value::List(vec![
                Value::Map(vec![("name".into(), s("x")), ("n".into(), Value::Number(1.0))]),
                Value::Map(vec![("name".into(), s("y"))]),
            ])
        );
    }

    #[test]
    fn unquoted_wikilink_list_is_a_string_list() {
        let p = parse("up: [[Parent]]\n").unwrap();
        assert_eq!(p[0].1, Value::List(vec![Value::List(vec![s("Parent")])]));
    }

    #[test]
    fn round_trip() {
        let src = "title: \"A: B\"\ntags:\n  - x\n  - \"#y\"\nn: 42\nflag: false\nnested:\n  k: v\n  l:\n    - 1\nitems:\n  - name: x\n    n: 1\nempty: []\nnone:\nmulti: \"a\\nb\"\nnumstr: \"12\"\n";
        let p = parse(src).unwrap();
        let out = to_string(&p);
        assert_eq!(parse(&out).unwrap(), p);
        assert_eq!(out, src);
    }

    #[test]
    fn errors_have_lines() {
        let e = parse("a: 1\nnot a mapping line\n").unwrap_err();
        assert_eq!(e.line, 2);
        assert!(parse("- a\n- b\n").is_err());
    }

    #[test]
    fn string_lists() {
        assert_eq!(s("a, b").as_string_list(), vec!["a", "b"]);
        assert_eq!(Value::List(vec![s("x"), Value::Number(2.0)]).as_string_list(), vec!["x", "2"]);
    }
}
