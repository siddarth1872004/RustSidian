//! CSS parser for the supported subset. Error tolerant: an unparsable rule or declaration is
//! skipped and reported, and parsing continues with the next one.

use crate::selector::{parse_selector_list, Selector};

#[derive(Clone, PartialEq, Debug)]
pub struct Declaration {
    pub name: Box<str>,
    pub value: Box<str>,
    pub important: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Media {
    Always,
    Dark,
    Light,
    ReducedMotion,
    /// A media query this engine does not evaluate (print, width queries). Never applies.
    Unsupported,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Rule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
    pub media: Media,
    /// 1-based line of the rule in its stylesheet, for the theme inspector.
    pub line: u32,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    pub line: u32,
    pub message: String,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
    pub diagnostics: Vec<Diagnostic>,
}

struct P<'a> {
    b: &'a [u8],
    s: &'a str,
    i: usize,
    out: Stylesheet,
}

pub fn parse(src: &str) -> Stylesheet {
    let mut p = P { b: src.as_bytes(), s: src, i: 0, out: Stylesheet::default() };
    p.rules(Media::Always, 0);
    p.out
}

/// Parse the body of a `style="…"` attribute or a block without braces.
pub fn parse_declarations(src: &str) -> Vec<Declaration> {
    let mut out = Vec::new();
    for part in split_top_level(src, b';') {
        if let Some(d) = declaration(part) {
            out.push(d);
        }
    }
    out
}

fn split_top_level(s: &str, sep: u8) -> Vec<&str> {
    let b = s.as_bytes();
    let (mut depth, mut quote, mut start) = (0i32, 0u8, 0);
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if quote != 0 {
            if c == b'\\' {
                i += 1;
            } else if c == quote {
                quote = 0;
            }
        } else if c == b'"' || c == b'\'' {
            quote = c;
        } else if c == b'(' {
            depth += 1;
        } else if c == b')' {
            depth -= 1;
        } else if c == sep && depth <= 0 {
            out.push(&s[start..i]);
            start = i + 1;
        }
        i += 1;
    }
    out.push(&s[start.min(s.len())..]);
    out
}

fn declaration(part: &str) -> Option<Declaration> {
    let (name, value) = part.split_once(':')?;
    let name = name.trim();
    let mut value = value.trim();
    let valid = !name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_');
    if !valid || value.is_empty() {
        return None;
    }
    let mut important = false;
    if let Some(v) = value.strip_suffix("!important").or_else(|| value.strip_suffix("! important")) {
        important = true;
        value = v.trim_end();
    }
    // Custom property names are case-sensitive; ordinary properties are not.
    let name: Box<str> = if name.starts_with("--") { name.into() } else { name.to_ascii_lowercase().into() };
    Some(Declaration { name, value: value.into(), important })
}

impl P<'_> {
    fn line(&self, at: usize) -> u32 {
        self.b[..at.min(self.b.len())].iter().filter(|&&c| c == b'\n').count() as u32 + 1
    }
    fn diag(&mut self, at: usize, m: &str) {
        if self.out.diagnostics.len() < 200 {
            let line = self.line(at);
            self.out.diagnostics.push(Diagnostic { line, message: m.to_string() });
        }
    }
    fn skip_ws(&mut self) {
        loop {
            while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
            if self.b[self.i..].starts_with(b"/*") {
                match self.s[self.i + 2..].find("*/") {
                    Some(p) => self.i += p + 4,
                    None => self.i = self.b.len(),
                }
            } else {
                break;
            }
        }
    }

    /// Scan to `stop` at nesting depth 0, honoring strings, comments and brackets.
    /// Returns the index of the stop byte, or the end of input.
    fn scan(&self, from: usize, stops: &[u8]) -> usize {
        let (mut depth, mut quote) = (0i32, 0u8);
        let mut i = from;
        while i < self.b.len() {
            let c = self.b[i];
            if quote != 0 {
                if c == b'\\' {
                    i += 1;
                } else if c == quote {
                    quote = 0;
                }
            } else if c == b'"' || c == b'\'' {
                quote = c;
            } else if self.b[i..].starts_with(b"/*") {
                i = self.s[i + 2..].find("*/").map_or(self.b.len(), |p| i + p + 3);
            } else if depth == 0 && stops.contains(&c) {
                return i;
            } else if matches!(c, b'(' | b'[') {
                depth += 1;
            } else if matches!(c, b')' | b']') {
                depth -= 1;
            }
            i += 1;
        }
        self.b.len()
    }

    /// Index just past the `}` matching the `{` at `open`.
    fn block_end(&self, open: usize) -> usize {
        let (mut depth, mut quote) = (0i32, 0u8);
        let mut i = open;
        while i < self.b.len() {
            let c = self.b[i];
            if quote != 0 {
                if c == b'\\' {
                    i += 1;
                } else if c == quote {
                    quote = 0;
                }
            } else if c == b'"' || c == b'\'' {
                quote = c;
            } else if self.b[i..].starts_with(b"/*") {
                i = self.s[i + 2..].find("*/").map_or(self.b.len(), |p| i + p + 3);
            } else if c == b'{' {
                depth += 1;
            } else if c == b'}' {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            i += 1;
        }
        self.b.len()
    }

    fn rules(&mut self, media: Media, nesting: u32) {
        loop {
            self.skip_ws();
            if self.i >= self.b.len() {
                return;
            }
            if self.b[self.i] == b'}' {
                self.i += 1;
                if nesting > 0 {
                    return;
                }
                continue;
            }
            let start = self.i;
            if self.b[self.i] == b'@' {
                self.at_rule(media, nesting);
                continue;
            }
            let open = self.scan(self.i, b"{};");
            if open >= self.b.len() || self.b[open] != b'{' {
                self.diag(start, "expected `{` after selector");
                self.i = (open + 1).min(self.b.len());
                continue;
            }
            let end = self.block_end(open);
            let prelude = &self.s[start..open];
            // An unclosed block runs to the end of the input and has no `}` to drop.
            let closed = end > open + 1 && self.b[end - 1] == b'}';
            let body = &self.s[open + 1..if closed { end - 1 } else { end }];
            self.i = end;
            let line = self.line(start);
            match parse_selector_list(&strip_comments(prelude)) {
                Ok(selectors) if !selectors.is_empty() => {
                    let body = strip_comments(body);
                    let mut declarations = Vec::new();
                    for part in split_top_level(&body, b';') {
                        if part.trim().is_empty() {
                            continue;
                        }
                        match declaration(part) {
                            Some(d) => declarations.push(d),
                            None => self.diag(
                                start,
                                &format!(
                                    "invalid declaration `{}`",
                                    part.trim().chars().take(60).collect::<String>()
                                ),
                            ),
                        }
                    }
                    self.out.rules.push(Rule { selectors, declarations, media, line });
                }
                Ok(_) => {}
                Err(e) => self.diag(
                    start,
                    &format!(
                        "unsupported selector `{}`: {e}",
                        prelude.trim().chars().take(80).collect::<String>()
                    ),
                ),
            }
        }
    }

    fn at_rule(&mut self, media: Media, nesting: u32) {
        let start = self.i;
        let stop = self.scan(self.i, b"{;");
        let prelude = self.s[start..stop].trim().to_ascii_lowercase();
        if stop >= self.b.len() || self.b[stop] == b';' {
            // @import, @charset, @namespace: ignored.
            self.i = (stop + 1).min(self.b.len());
            return;
        }
        if let Some(query) = prelude.strip_prefix("@media") {
            if nesting > 8 {
                self.i = self.block_end(stop);
                return;
            }
            let q: String = query.chars().filter(|c| !c.is_whitespace()).collect();
            let inner = match q.as_str() {
                "(prefers-color-scheme:dark)" => Media::Dark,
                "(prefers-color-scheme:light)" => Media::Light,
                "(prefers-reduced-motion:reduce)" => Media::ReducedMotion,
                "all" | "screen" => Media::Always,
                _ => Media::Unsupported,
            };
            // Nested media: the inner condition wins unless the outer one can never apply.
            let combined = if media == Media::Always || media == inner { inner } else { Media::Unsupported };
            self.i = stop + 1;
            self.rules(combined, nesting + 1);
        } else {
            // @font-face, @keyframes, @supports, @layer …: skipped as a block.
            if !prelude.starts_with("@font-face")
                && !prelude.starts_with("@keyframes")
                && !prelude.starts_with("@-")
            {
                self.diag(
                    start,
                    &format!("unsupported at-rule `{}`", prelude.chars().take(40).collect::<String>()),
                );
            }
            self.i = self.block_end(stop);
        }
    }
}

fn strip_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find("/*") {
        out.push_str(&rest[..p]);
        match rest[p + 2..].find("*/") {
            Some(q) => rest = &rest[p + 2 + q + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_and_declarations() {
        let s = parse("/* c */ .a, body > .b:hover { color: red; --x: calc(1px + 2px) ; background: url(\"a;b{}\") !important }\n.c{}\n");
        assert!(s.diagnostics.is_empty(), "{:?}", s.diagnostics);
        assert_eq!(s.rules.len(), 2);
        assert_eq!(s.rules[0].selectors.len(), 2);
        let d = &s.rules[0].declarations;
        assert_eq!((&*d[0].name, &*d[0].value, d[0].important), ("color", "red", false));
        assert_eq!((&*d[1].name, &*d[1].value), ("--x", "calc(1px + 2px)"));
        assert_eq!((&*d[2].value, d[2].important), ("url(\"a;b{}\")", true));
        assert_eq!(s.rules[1].line, 2);
    }

    #[test]
    fn media_queries() {
        let s = parse("@media (prefers-color-scheme: dark) { .a { color: #fff } .b { color: #000 } }\n@media print { .p { x: y } }\n.z { a: b }\n@media (prefers-reduced-motion: reduce) { * { transition: none } }");
        let media: Vec<Media> = s.rules.iter().map(|r| r.media).collect();
        assert_eq!(
            media,
            vec![Media::Dark, Media::Dark, Media::Unsupported, Media::Always, Media::ReducedMotion]
        );
    }

    #[test]
    fn recovers_from_errors() {
        let s = parse(".ok { color: red; bogus; : x; width: 1px }\n.a::before { content: 'x' }\n@import url(x.css);\n@font-face { font-family: X; }\n@keyframes k { from { a: b } to { a: c } }\n}} .after { top: 0 }\n.unclosed { color: blue");
        let names: Vec<String> = s
            .rules
            .iter()
            .map(|r| format!("{:?}", r.declarations.iter().map(|d| &*d.name).collect::<Vec<_>>()))
            .collect();
        assert_eq!(names, vec!["[\"color\", \"width\"]", "[\"top\"]", "[\"color\"]"]);
        assert!(s.diagnostics.iter().any(|d| d.message.contains("bogus")));
        assert!(s.diagnostics.iter().any(|d| d.message.contains("::before") && d.line == 2));
    }

    #[test]
    fn inline_declarations() {
        let d = parse_declarations("color: red; margin:0 auto ;; Font-Weight: BOLD");
        assert_eq!(d.len(), 3);
        assert_eq!((&*d[2].name, &*d[2].value), ("font-weight", "BOLD"));
    }
}
