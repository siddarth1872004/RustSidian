//! Cascade: several stylesheets, specificity, `!important`, custom properties with `var()`,
//! inheritance, shorthand expansion, and an inspector listing the rules that match an element.

use crate::css::{self, Declaration, Media, Stylesheet};
use crate::selector::{Element, Simple};
use crate::value::{parse_color, parse_length, Color, LengthCx};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Env {
    pub dark: bool,
    pub reduced_motion: bool,
}

struct Entry {
    sheet: u16,
    rule: u32,
    selector: u16,
}

#[derive(Default)]
pub struct Theme {
    sheets: Vec<(String, Stylesheet)>,
    by_class: HashMap<Box<str>, Vec<Entry>>,
    by_id: HashMap<Box<str>, Vec<Entry>>,
    by_tag: HashMap<Box<str>, Vec<Entry>>,
    universal: Vec<Entry>,
}

/// A rule that applies to an element, for the theme inspector.
#[derive(Clone, PartialEq, Debug)]
pub struct MatchedRule {
    pub sheet: String,
    pub line: u32,
    pub specificity: u32,
    pub declarations: Vec<Declaration>,
}

#[derive(Clone, Debug, Default)]
pub struct Computed {
    /// Custom properties visible to this element (own and inherited), unresolved.
    pub vars: Arc<HashMap<Box<str>, Box<str>>>,
    /// Ordinary properties with `var()` substituted.
    pub props: HashMap<Box<str>, Box<str>>,
}

const INHERITED: &[&str] = &[
    "color",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "line-height",
    "letter-spacing",
    "text-align",
    "cursor",
    "visibility",
    "white-space",
    "direction",
    "tab-size",
    "caret-color",
];

fn side_values(v: &str) -> Option<[&str; 4]> {
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0, 0);
    for (i, c) in v.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            c if c.is_whitespace() && depth == 0 => {
                if i > start {
                    parts.push(&v[start..i]);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < v.len() {
        parts.push(&v[start..]);
    }
    Some(match parts.as_slice() {
        [a] => [a, a, a, a],
        [a, b] => [a, b, a, b],
        [a, b, c] => [a, b, c, b],
        [a, b, c, d] => [a, b, c, d],
        _ => return None,
    })
}

/// Expand shorthands into the longhands the UI reads.
fn expand(name: &str, value: &str, out: &mut Vec<(Box<str>, Box<str>)>) {
    match name {
        "margin" | "padding" => {
            if let Some(v) = side_values(value) {
                for (side, val) in ["top", "right", "bottom", "left"].iter().zip(v) {
                    out.push((format!("{name}-{side}").into(), val.into()));
                }
                return;
            }
        }
        "border" | "border-top" | "border-right" | "border-bottom" | "border-left" => {
            let sides: &[&str] =
                if name == "border" { &["top", "right", "bottom", "left"] } else { &[&name[7..]] };
            let (mut width, mut style, mut color) = ("medium", "none", "currentcolor");
            let tokens = split_ws(value);
            for tok in &tokens {
                if matches!(*tok, "none" | "solid" | "dashed" | "dotted" | "double" | "hidden") {
                    style = tok;
                } else if tok.starts_with(|c: char| c.is_ascii_digit() || c == '.')
                    || matches!(*tok, "thin" | "medium" | "thick")
                    || tok.starts_with("calc(")
                {
                    width = tok;
                } else {
                    color = tok;
                }
            }
            let width = match width {
                "thin" => "1px",
                "medium" => "3px",
                "thick" => "5px",
                w => w,
            };
            let width = if style == "none" || style == "hidden" { "0" } else { width };
            for s in sides {
                out.push((format!("border-{s}-width").into(), width.into()));
                out.push((format!("border-{s}-style").into(), style.into()));
                out.push((format!("border-{s}-color").into(), color.into()));
            }
            return;
        }
        "border-width" | "border-color" | "border-style" => {
            if let Some(v) = side_values(value) {
                let what = &name[7..];
                for (side, val) in ["top", "right", "bottom", "left"].iter().zip(v) {
                    out.push((format!("border-{side}-{what}").into(), val.into()));
                }
                return;
            }
        }
        "background" => {
            // Only the color component is used; images and gradients are not rendered.
            if let Some(c) = split_ws(value).into_iter().find(|t| parse_color(t).is_some()) {
                out.push(("background-color".into(), c.into()));
                return;
            }
            if value.trim() == "none" {
                out.push(("background-color".into(), "transparent".into()));
                return;
            }
        }
        "gap" => {
            let t = split_ws(value);
            if let Some(first) = t.first() {
                out.push(("row-gap".into(), (*first).into()));
                out.push(("column-gap".into(), (*t.get(1).unwrap_or(first)).into()));
                return;
            }
        }
        "overflow" => {
            let t = split_ws(value);
            if let Some(first) = t.first() {
                out.push(("overflow-x".into(), (*first).into()));
                out.push(("overflow-y".into(), (*t.get(1).unwrap_or(first)).into()));
                return;
            }
        }
        _ => {}
    }
    out.push((name.into(), value.into()));
}

fn split_ws(v: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0, 0);
    for (i, c) in v.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            c if c.is_whitespace() && depth == 0 => {
                if i > start {
                    parts.push(&v[start..i]);
                }
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    if start < v.len() {
        parts.push(&v[start..]);
    }
    parts
}

/// Substitute `var(--name, fallback)` using `vars`. Returns `None` if a variable without a
/// fallback is missing or a reference cycle is found (the declaration is then invalid).
pub fn resolve_vars(value: &str, vars: &HashMap<Box<str>, Box<str>>, depth: u32) -> Option<String> {
    if !value.contains("var(") {
        return Some(value.to_string());
    }
    if depth > 24 {
        return None;
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(p) = rest.find("var(") {
        out.push_str(&rest[..p]);
        let after = &rest[p + 4..];
        let mut d = 1;
        let mut close = None;
        for (i, c) in after.char_indices() {
            match c {
                '(' => d += 1,
                ')' => {
                    d -= 1;
                    if d == 0 {
                        close = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = close?;
        let inner = &after[..close];
        let (name, fallback) = match inner.find(',') {
            Some(c) => (inner[..c].trim(), Some(inner[c + 1..].trim())),
            None => (inner.trim(), None),
        };
        let resolved = match vars.get(name) {
            Some(v) => resolve_vars(v, vars, depth + 1),
            None => None,
        };
        out.push_str(&resolved.or_else(|| fallback.and_then(|f| resolve_vars(f, vars, depth + 1)))?);
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    Some(out)
}

impl Theme {
    pub fn new() -> Theme {
        Theme::default()
    }

    /// Add a stylesheet. Later sheets win over earlier ones at equal specificity, so add the
    /// built-in base first, then the theme, then snippets.
    pub fn add_sheet(&mut self, name: &str, source: &str) -> &Stylesheet {
        let sheet = css::parse(source);
        let si = self.sheets.len() as u16;
        for (ri, rule) in sheet.rules.iter().enumerate() {
            for (seli, sel) in rule.selectors.iter().enumerate() {
                let e = Entry { sheet: si, rule: ri as u32, selector: seli as u16 };
                match sel.key() {
                    Some(Simple::Id(i)) => self.by_id.entry(i.clone()).or_default().push(e),
                    Some(Simple::Class(c)) => self.by_class.entry(c.clone()).or_default().push(e),
                    Some(Simple::Tag(t)) => self.by_tag.entry(t.clone()).or_default().push(e),
                    _ => self.universal.push(e),
                }
            }
        }
        self.sheets.push((name.to_string(), sheet));
        &self.sheets[si as usize].1
    }

    pub fn sheet_names(&self) -> impl Iterator<Item = &str> {
        self.sheets.iter().map(|(n, _)| n.as_str())
    }

    /// Matching (sheet, rule, specificity) triples, lowest priority first.
    fn matching<E: Element>(&self, e: &E, env: Env, classes: &[&str]) -> Vec<(u16, u32, u32)> {
        let mut found: Vec<(u16, u32, u32)> = Vec::new();
        let mut check = |entries: &Vec<Entry>| {
            for en in entries {
                let rule = &self.sheets[en.sheet as usize].1.rules[en.rule as usize];
                let applies = match rule.media {
                    Media::Always => true,
                    Media::Dark => env.dark,
                    Media::Light => !env.dark,
                    Media::ReducedMotion => env.reduced_motion,
                    Media::Unsupported => false,
                };
                let sel = &rule.selectors[en.selector as usize];
                if applies && sel.matches(e) {
                    found.push((en.sheet, en.rule, sel.specificity));
                }
            }
        };
        check(&self.universal);
        if let Some(v) = self.by_tag.get(e.tag().to_ascii_lowercase().as_str()) {
            check(v);
        }
        if let Some(v) = e.id().and_then(|i| self.by_id.get(i)) {
            check(v);
        }
        for c in classes {
            if let Some(v) = self.by_class.get(*c) {
                check(v);
            }
        }
        // (specificity, sheet, rule) ascending = cascade order within one importance level.
        found.sort_by_key(|&(s, r, spec)| (spec, s, r));
        // A rule with several matching selectors counts once, at its highest specificity.
        let mut out: Vec<(u16, u32, u32)> = Vec::with_capacity(found.len());
        for f in found.into_iter().rev() {
            if !out.iter().any(|o| o.0 == f.0 && o.1 == f.1) {
                out.push(f);
            }
        }
        out.reverse();
        out
    }

    /// Compute the style of an element. `classes` are the element's classes (used to find
    /// candidate rules quickly); `inline` is an optional `style` attribute.
    pub fn compute<E: Element>(
        &self,
        e: &E,
        classes: &[&str],
        env: Env,
        parent: Option<&Computed>,
        inline: Option<&str>,
    ) -> Computed {
        let matched = self.matching(e, env, classes);
        // Declarations in cascade order: normal ones first, then `!important` ones.
        let mut normal: Vec<(Box<str>, Box<str>)> = Vec::new();
        let mut important: Vec<(Box<str>, Box<str>)> = Vec::new();
        let inline_decls = inline.map(css::parse_declarations).unwrap_or_default();
        let rule_decls = matched
            .iter()
            .flat_map(|&(s, r, _)| self.sheets[s as usize].1.rules[r as usize].declarations.iter());
        for d in rule_decls.chain(inline_decls.iter()) {
            let target = if d.important { &mut important } else { &mut normal };
            target.push((d.name.clone(), d.value.clone()));
        }
        normal.append(&mut important);

        let own_vars = normal.iter().filter(|(k, _)| k.starts_with("--")).count();
        let vars = match (parent, own_vars) {
            (Some(p), 0) => p.vars.clone(),
            (p, _) => {
                let mut v: HashMap<Box<str>, Box<str>> = p.map(|p| (*p.vars).clone()).unwrap_or_default();
                for (k, val) in normal.iter().filter(|(k, _)| k.starts_with("--")) {
                    v.insert(k.clone(), val.clone());
                }
                Arc::new(v)
            }
        };
        let mut props: HashMap<Box<str>, Box<str>> = HashMap::new();
        if let Some(p) = parent {
            for name in INHERITED {
                if let Some(v) = p.props.get(*name) {
                    props.insert((*name).into(), v.clone());
                }
            }
        }
        let mut longhands = Vec::new();
        for (k, v) in normal.iter().filter(|(k, _)| !k.starts_with("--")) {
            // Variables are substituted before shorthands are expanded, so
            // `background: var(--bg)` and `padding: var(--a) var(--b)` work.
            // An unresolvable var() invalidates the declaration; the previous value stays.
            let Some(resolved) = resolve_vars(v, &vars, 0) else { continue };
            longhands.clear();
            expand(k, resolved.trim(), &mut longhands);
            for (name, value) in longhands.drain(..) {
                match &*value {
                    "inherit" => match parent.and_then(|p| p.props.get(&name)) {
                        Some(pv) => {
                            props.insert(name, pv.clone());
                        }
                        None => {
                            props.remove(&name);
                        }
                    },
                    "initial" | "unset" | "revert" => {
                        props.remove(&name);
                    }
                    _ => {
                        props.insert(name, value);
                    }
                }
            }
        }
        Computed { vars, props }
    }

    /// The rules that match an element, highest priority first.
    pub fn inspect<E: Element>(&self, e: &E, classes: &[&str], env: Env) -> Vec<MatchedRule> {
        let mut out: Vec<MatchedRule> = self
            .matching(e, env, classes)
            .into_iter()
            .map(|(s, r, specificity)| {
                let (name, sheet) = &self.sheets[s as usize];
                let rule = &sheet.rules[r as usize];
                MatchedRule {
                    sheet: name.clone(),
                    line: rule.line,
                    specificity,
                    declarations: rule.declarations.clone(),
                }
            })
            .collect();
        out.reverse();
        out
    }
}

impl Computed {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.props.get(name).map(|v| &**v)
    }
    /// A custom property, fully resolved.
    pub fn var(&self, name: &str) -> Option<String> {
        resolve_vars(self.vars.get(name)?, &self.vars, 0)
    }
    pub fn color(&self, name: &str) -> Option<Color> {
        let v = self.get(name)?;
        if v.eq_ignore_ascii_case("currentcolor") {
            return self.get("color").filter(|_| name != "color").and_then(parse_color);
        }
        parse_color(v)
    }
    pub fn length(&self, name: &str, cx: &LengthCx) -> Option<f32> {
        parse_length(self.get(name)?, cx)
    }
    pub fn number(&self, name: &str) -> Option<f32> {
        self.get(name)?.trim().parse().ok()
    }
    /// `font-weight` as a number (`bold` = 700, `normal` = 400).
    pub fn font_weight(&self) -> u16 {
        match self.get("font-weight").map(str::trim) {
            Some("bold") => 700,
            Some("bolder") => 800,
            Some("lighter") => 300,
            Some(n) => n.parse().unwrap_or(400),
            None => 400,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selector::tests::Dom;

    fn setup() -> (Dom, usize, usize, usize) {
        let mut d = Dom::default();
        let body = d.add(None, "body.theme-dark");
        let pane = d.add(Some(body), "div.pane");
        let link = d.add(Some(pane), "a#home.link.is-active");
        (d, body, pane, link)
    }

    fn classes(d: &Dom, i: usize) -> Vec<&str> {
        d.nodes[i].classes.iter().map(String::as_str).collect()
    }

    fn compute_chain(t: &Theme, d: &Dom, env: Env, chain: &[usize]) -> Computed {
        let mut parent: Option<Computed> = None;
        for &i in chain {
            parent = Some(t.compute(&d.el(i), &classes(d, i), env, parent.as_ref(), None));
        }
        parent.unwrap()
    }

    #[test]
    fn specificity_order_and_important() {
        let (d, body, pane, link) = setup();
        let mut t = Theme::new();
        t.add_sheet("base", "a { color: red; width: 10px } .link { color: green } #home { color: blue } .link.is-active { color: yellow; width: 20px !important }");
        t.add_sheet("snippet", ".link { color: purple; width: 30px } a { height: 5px }");
        let c = compute_chain(&t, &d, Env::default(), &[body, pane, link]);
        assert_eq!(c.get("color"), Some("blue"), "id beats classes");
        assert_eq!(c.get("width"), Some("20px"), "!important beats a later sheet");
        assert_eq!(c.get("height"), Some("5px"));
        let rules = t.inspect(&d.el(link), &classes(&d, link), Env::default());
        assert_eq!(rules[0].declarations[0].value.as_ref(), "blue");
        assert_eq!((rules.len(), rules.last().unwrap().sheet.as_str()), (6, "base"));
    }

    #[test]
    fn later_sheet_wins_at_equal_specificity() {
        let (d, body, pane, link) = setup();
        let mut t = Theme::new();
        t.add_sheet("base", ".link { color: green }");
        t.add_sheet("theme", ".link { color: purple }");
        assert_eq!(compute_chain(&t, &d, Env::default(), &[body, pane, link]).get("color"), Some("purple"));
    }

    #[test]
    fn variables_inherit_and_resolve() {
        let (d, body, pane, link) = setup();
        let mut t = Theme::new();
        t.add_sheet(
            "theme",
            "body { --accent-h: 258; --accent: hsl(var(--accent-h), 88%, 66%); --bg: #202020; --pad: 4px; color: var(--text, #ddd); font-size: 16px }
             .theme-dark { --bg: #1e1e1e }
             .pane { --pad: 8px; background: var(--bg); padding: var(--pad) calc(var(--pad) * 2) }
             a { color: var(--accent); border: 1px solid var(--missing, var(--accent)); margin-left: var(--nope); outline-color: var(--loop) }
             a { --loop: var(--loop2); --loop2: var(--loop) }",
        );
        let c = compute_chain(&t, &d, Env::default(), &[body, pane, link]);
        assert_eq!(c.color("color"), Some(Color::rgb(138, 92, 245)));
        assert_eq!(c.get("border-top-width"), Some("1px"));
        assert_eq!(c.color("border-left-color"), Some(Color::rgb(138, 92, 245)), "nested fallback");
        assert_eq!(c.get("margin-left"), None, "missing variable invalidates the declaration");
        assert_eq!(c.get("outline-color"), None, "cycles are invalid, not infinite");
        assert_eq!(c.get("font-size"), Some("16px"), "inherited property");
        assert_eq!(c.var("--pad").as_deref(), Some("8px"), "custom properties inherit and can be overridden");
        let p = compute_chain(&t, &d, Env::default(), &[body, pane]);
        assert_eq!(p.color("background-color"), Some(Color::rgb(30, 30, 30)), ".theme-dark overrides body");
        let cx = LengthCx::default();
        assert_eq!(
            (p.length("padding-top", &cx), p.length("padding-right", &cx), p.length("padding-left", &cx)),
            (Some(8.0), Some(16.0), Some(16.0))
        );
        assert_eq!(p.get("color"), Some("#ddd"));
    }

    #[test]
    fn media_and_inline_and_keywords() {
        let (d, body, pane, link) = setup();
        let mut t = Theme::new();
        t.add_sheet("s", "body { color: black; font-weight: bold } @media (prefers-color-scheme: dark) { body { color: white } } .pane { color: initial; font-weight: inherit } a { transition: color 100ms } @media (prefers-reduced-motion: reduce) { a { transition: none } }");
        let dark = Env { dark: true, reduced_motion: false };
        assert_eq!(compute_chain(&t, &d, dark, &[body]).get("color"), Some("white"));
        assert_eq!(compute_chain(&t, &d, Env::default(), &[body]).get("color"), Some("black"));
        let p = compute_chain(&t, &d, dark, &[body, pane]);
        assert_eq!((p.get("color"), p.font_weight()), (None, 700));
        let b = t.compute(&d.el(body), &classes(&d, body), dark, None, None);
        let inline =
            t.compute(&d.el(pane), &classes(&d, pane), dark, Some(&b), Some("color: #abc; margin: 1px 2px"));
        assert_eq!(
            (inline.get("color"), inline.get("margin-bottom"), inline.get("margin-left")),
            (Some("#abc"), Some("1px"), Some("2px"))
        );
        let motion = compute_chain(&t, &d, Env { dark: false, reduced_motion: true }, &[body, pane, link]);
        assert_eq!(motion.get("transition"), Some("none"));
    }

    #[test]
    fn shorthands() {
        let mut out = Vec::new();
        expand("border-bottom", "2px dashed red", &mut out);
        expand("border", "none", &mut out);
        expand("gap", "4px", &mut out);
        expand("background", "url(x.png) no-repeat #123", &mut out);
        let get = |k: &str| out.iter().rev().find(|(n, _)| &**n == k).map(|(_, v)| v.to_string());
        assert_eq!(get("border-bottom-style").as_deref(), Some("none"));
        assert_eq!(get("border-top-width").as_deref(), Some("0"));
        assert_eq!(get("column-gap").as_deref(), Some("4px"));
        assert_eq!(get("background-color").as_deref(), Some("#123"));
        assert_eq!(out[0], ("border-bottom-width".into(), "2px".into()));
    }
}
