//! Selectors: parsing, specificity, matching against an abstract element tree.

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AttrOp {
    Exists,
    Eq,
    Prefix,
    Suffix,
    Contains,
    /// `~=`: whitespace-separated word.
    Word,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Simple {
    Universal,
    Tag(Box<str>),
    Id(Box<str>),
    Class(Box<str>),
    Attr {
        name: Box<str>,
        op: AttrOp,
        value: Box<str>,
    },
    Hover,
    Focus,
    Active,
    Disabled,
    Checked,
    FirstChild,
    LastChild,
    Root,
    Not(Vec<Simple>),
    /// `:is(...)` / `:where(...)`: any of the compounds.
    Is(Vec<Vec<Simple>>, bool),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Combinator {
    Descendant,
    Child,
    Adjacent,
    Sibling,
}

/// A complex selector, stored right to left: `parts[0]` is the subject compound.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Selector {
    pub parts: Vec<(Vec<Simple>, Combinator)>,
    pub specificity: u32,
}

/// What a widget must expose for selector matching.
pub trait Element: Sized {
    fn tag(&self) -> &str;
    fn id(&self) -> Option<&str>;
    fn has_class(&self, class: &str) -> bool;
    fn attr(&self, name: &str) -> Option<&str>;
    fn parent(&self) -> Option<Self>;
    fn prev_sibling(&self) -> Option<Self>;
    fn is_last_child(&self) -> bool;
    fn hover(&self) -> bool {
        false
    }
    fn focus(&self) -> bool {
        false
    }
    fn active(&self) -> bool {
        false
    }
    fn disabled(&self) -> bool {
        false
    }
    fn checked(&self) -> bool {
        false
    }
}

fn ident_end(s: &str) -> usize {
    let mut end = 0;
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        if c == '\\' {
            if let Some((j, n)) = chars.next() {
                end = j + n.len_utf8();
                continue;
            }
        }
        if c.is_alphanumeric() || c == '-' || c == '_' || !c.is_ascii() {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    end
}

fn unescape(s: &str) -> Box<str> {
    s.replace('\\', "").into()
}

fn matching_paren(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
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

fn parse_compound(mut s: &str) -> Result<(Vec<Simple>, &str), String> {
    let mut out = Vec::new();
    while let Some(c) = s.chars().next() {
        match c {
            '*' => {
                out.push(Simple::Universal);
                s = &s[1..];
            }
            '.' | '#' => {
                let n = ident_end(&s[1..]);
                if n == 0 {
                    return Err("empty class or id".into());
                }
                let name = unescape(&s[1..1 + n]);
                out.push(if c == '.' { Simple::Class(name) } else { Simple::Id(name) });
                s = &s[1 + n..];
            }
            '[' => {
                let close = s.find(']').ok_or("unclosed `[`")?;
                let inner = &s[1..close];
                s = &s[close + 1..];
                let ops = [
                    ("~=", AttrOp::Word),
                    ("^=", AttrOp::Prefix),
                    ("$=", AttrOp::Suffix),
                    ("*=", AttrOp::Contains),
                    ("=", AttrOp::Eq),
                ];
                let found = ops
                    .iter()
                    .filter_map(|(t, op)| inner.find(t).map(|p| (p, *t, op.clone())))
                    .min_by_key(|x| x.0);
                match found {
                    Some((p, t, op)) => {
                        let value = inner[p + t.len()..].trim();
                        let value =
                            value.strip_suffix(" i").unwrap_or(value).trim().trim_matches(['"', '\'']);
                        out.push(Simple::Attr { name: inner[..p].trim().into(), op, value: value.into() });
                    }
                    None => out.push(Simple::Attr {
                        name: inner.trim().into(),
                        op: AttrOp::Exists,
                        value: "".into(),
                    }),
                }
            }
            ':' => {
                if s.starts_with("::") {
                    return Err("pseudo-elements are not supported".into());
                }
                let n = ident_end(&s[1..]);
                let name = s[1..1 + n].to_ascii_lowercase();
                s = &s[1 + n..];
                let mut arg = None;
                if s.starts_with('(') {
                    let close = matching_paren(s).ok_or("unclosed `(`")?;
                    arg = Some(&s[1..close]);
                    s = &s[close + 1..];
                }
                out.push(match (name.as_str(), arg) {
                    ("hover", None) => Simple::Hover,
                    ("focus" | "focus-visible" | "focus-within", None) => Simple::Focus,
                    ("active", None) => Simple::Active,
                    ("disabled", None) => Simple::Disabled,
                    ("checked", None) => Simple::Checked,
                    ("first-child", None) => Simple::FirstChild,
                    ("last-child", None) => Simple::LastChild,
                    ("root", None) => Simple::Root,
                    ("not", Some(a)) => {
                        let (inner, rest) = parse_compound(a.trim())?;
                        if !rest.trim().is_empty() || inner.is_empty() {
                            return Err(":not() supports a single compound selector".into());
                        }
                        Simple::Not(inner)
                    }
                    ("is" | "where", Some(a)) => {
                        let mut alts = Vec::new();
                        for part in a.split(',') {
                            let (inner, rest) = parse_compound(part.trim())?;
                            if !rest.trim().is_empty() || inner.is_empty() {
                                return Err(":is() supports compound selectors only".into());
                            }
                            alts.push(inner);
                        }
                        Simple::Is(alts, name == "where")
                    }
                    (other, _) => return Err(format!("unsupported pseudo-class `:{other}`")),
                });
            }
            c if c.is_alphabetic() || c == '-' || c == '_' => {
                let n = ident_end(s);
                out.push(Simple::Tag(s[..n].to_ascii_lowercase().into()));
                s = &s[n..];
            }
            _ => break,
        }
    }
    Ok((out, s))
}

fn specificity(parts: &[Simple]) -> u32 {
    parts
        .iter()
        .map(|p| match p {
            Simple::Id(_) => 0x10000,
            Simple::Universal => 0,
            Simple::Tag(_) => 1,
            Simple::Not(inner) => specificity(inner),
            Simple::Is(_, true) => 0,
            Simple::Is(alts, false) => alts.iter().map(|a| specificity(a)).max().unwrap_or(0),
            _ => 0x100,
        })
        .sum()
}

pub fn parse_selector(src: &str) -> Result<Selector, String> {
    let mut s = src.trim();
    let mut chain: Vec<(Vec<Simple>, Combinator)> = Vec::new();
    let mut pending = Combinator::Descendant;
    let mut spec = 0;
    loop {
        let (compound, rest) = parse_compound(s)?;
        if compound.is_empty() {
            return Err(format!("unexpected `{}`", rest.chars().take(12).collect::<String>()));
        }
        spec += specificity(&compound);
        // The combinator stored with a compound relates it to the compound on its left.
        chain.push((compound, pending));
        let trimmed = rest.trim_start();
        if trimmed.is_empty() {
            break;
        }
        let (comb, after) = match trimmed.as_bytes()[0] {
            b'>' => (Combinator::Child, &trimmed[1..]),
            b'+' => (Combinator::Adjacent, &trimmed[1..]),
            b'~' => (Combinator::Sibling, &trimmed[1..]),
            _ if trimmed.len() < rest.len() => (Combinator::Descendant, trimmed),
            _ => return Err(format!("unexpected `{}`", trimmed.chars().take(12).collect::<String>())),
        };
        pending = comb;
        s = after.trim_start();
    }
    if chain.len() > 64 {
        return Err("selector too long".into());
    }
    chain.reverse();
    Ok(Selector { parts: chain, specificity: spec })
}

pub fn parse_selector_list(src: &str) -> Result<Vec<Selector>, String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in src.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                out.push(parse_selector(&src[start..i])?);
                start = i + 1;
            }
            _ => {}
        }
    }
    if !src[start..].trim().is_empty() {
        out.push(parse_selector(&src[start..])?);
    }
    Ok(out)
}

fn matches_simple<E: Element>(s: &Simple, e: &E) -> bool {
    match s {
        Simple::Universal => true,
        Simple::Tag(t) => e.tag().eq_ignore_ascii_case(t),
        Simple::Id(i) => e.id() == Some(i),
        Simple::Class(c) => e.has_class(c),
        Simple::Attr { name, op, value } => match e.attr(name) {
            None => false,
            Some(v) => match op {
                AttrOp::Exists => true,
                AttrOp::Eq => v == &**value,
                AttrOp::Prefix => !value.is_empty() && v.starts_with(&**value),
                AttrOp::Suffix => !value.is_empty() && v.ends_with(&**value),
                AttrOp::Contains => !value.is_empty() && v.contains(&**value),
                AttrOp::Word => v.split_whitespace().any(|w| w == &**value),
            },
        },
        Simple::Hover => e.hover(),
        Simple::Focus => e.focus(),
        Simple::Active => e.active(),
        Simple::Disabled => e.disabled(),
        Simple::Checked => e.checked(),
        Simple::FirstChild => e.prev_sibling().is_none() && e.parent().is_some(),
        Simple::LastChild => e.is_last_child(),
        Simple::Root => e.parent().is_none(),
        Simple::Not(inner) => !inner.iter().all(|s| matches_simple(s, e)),
        Simple::Is(alts, _) => alts.iter().any(|a| a.iter().all(|s| matches_simple(s, e))),
    }
}

fn matches_from<E: Element>(parts: &[(Vec<Simple>, Combinator)], e: &E) -> bool {
    let Some(((compound, comb), rest)) = parts.split_first() else { return true };
    if !compound.iter().all(|s| matches_simple(s, e)) {
        return false;
    }
    if rest.is_empty() {
        return true;
    }
    match comb {
        Combinator::Child => e.parent().is_some_and(|p| matches_from(rest, &p)),
        Combinator::Adjacent => e.prev_sibling().is_some_and(|p| matches_from(rest, &p)),
        Combinator::Descendant => {
            let mut cur = e.parent();
            while let Some(p) = cur {
                if matches_from(rest, &p) {
                    return true;
                }
                cur = p.parent();
            }
            false
        }
        Combinator::Sibling => {
            let mut cur = e.prev_sibling();
            while let Some(p) = cur {
                if matches_from(rest, &p) {
                    return true;
                }
                cur = p.prev_sibling();
            }
            false
        }
    }
}

impl Selector {
    pub fn matches<E: Element>(&self, e: &E) -> bool {
        matches_from(&self.parts, e)
    }

    /// A class, id or tag the subject element must have; used to bucket rules.
    pub fn key(&self) -> Option<&Simple> {
        let subject = &self.parts.first()?.0;
        subject
            .iter()
            .find(|s| matches!(s, Simple::Id(_)))
            .or_else(|| subject.iter().find(|s| matches!(s, Simple::Class(_))))
            .or_else(|| subject.iter().find(|s| matches!(s, Simple::Tag(_))))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Minimal element tree for tests: nodes in a Vec with parent indices.
    #[derive(Default)]
    pub struct Dom {
        pub nodes: Vec<Node>,
    }
    #[derive(Default)]
    pub struct Node {
        pub tag: String,
        pub id: Option<String>,
        pub classes: Vec<String>,
        pub attrs: Vec<(String, String)>,
        pub parent: Option<usize>,
        pub hover: bool,
    }
    #[derive(Clone, Copy)]
    pub struct El<'a>(pub &'a Dom, pub usize);

    impl Dom {
        /// `spec` like `div#id.a.b[data-x=1]`.
        pub fn add(&mut self, parent: Option<usize>, spec: &str) -> usize {
            let mut n = Node { parent, ..Node::default() };
            let (head, attrs) = spec.split_once('[').unwrap_or((spec, ""));
            for a in attrs.trim_end_matches(']').split("][").filter(|a| !a.is_empty()) {
                let (k, v) = a.split_once('=').unwrap_or((a, ""));
                n.attrs.push((k.to_string(), v.to_string()));
            }
            let mut parts = head.split('.');
            let first = parts.next().unwrap_or("");
            let (tag, id) = first.split_once('#').map_or((first, None), |(t, i)| (t, Some(i.to_string())));
            n.tag = tag.to_string();
            n.id = id;
            n.classes = parts.map(str::to_string).collect();
            self.nodes.push(n);
            self.nodes.len() - 1
        }
        pub fn el(&self, i: usize) -> El<'_> {
            El(self, i)
        }
    }

    impl<'a> Element for El<'a> {
        fn tag(&self) -> &str {
            &self.0.nodes[self.1].tag
        }
        fn id(&self) -> Option<&str> {
            self.0.nodes[self.1].id.as_deref()
        }
        fn has_class(&self, c: &str) -> bool {
            self.0.nodes[self.1].classes.iter().any(|x| x == c)
        }
        fn attr(&self, name: &str) -> Option<&str> {
            self.0.nodes[self.1].attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
        }
        fn parent(&self) -> Option<Self> {
            self.0.nodes[self.1].parent.map(|p| El(self.0, p))
        }
        fn prev_sibling(&self) -> Option<Self> {
            let parent = self.0.nodes[self.1].parent;
            (0..self.1)
                .rev()
                .find(|&i| self.0.nodes[i].parent == parent && parent.is_some())
                .map(|i| El(self.0, i))
        }
        fn is_last_child(&self) -> bool {
            let parent = self.0.nodes[self.1].parent;
            parent.is_some() && !(self.1 + 1..self.0.nodes.len()).any(|i| self.0.nodes[i].parent == parent)
        }
        fn hover(&self) -> bool {
            self.0.nodes[self.1].hover
        }
    }

    fn dom() -> (Dom, [usize; 6]) {
        let mut d = Dom::default();
        let body = d.add(None, "body.theme-dark");
        let ws = d.add(Some(body), "div.workspace");
        let leaf = d.add(Some(ws), "div.workspace-leaf.mod-active[data-type=markdown]");
        let h1 = d.add(Some(leaf), "h1#title.cm-header");
        let p = d.add(Some(leaf), "p");
        let a = d.add(Some(p), "a.internal-link.is-unresolved[href=Some Note]");
        (d, [body, ws, leaf, h1, p, a])
    }

    fn m(d: &Dom, sel: &str, el: usize) -> bool {
        parse_selector(sel).unwrap_or_else(|e| panic!("{sel}: {e}")).matches(&d.el(el))
    }

    #[test]
    fn matching() {
        let (mut d, [body, ws, leaf, h1, p, a]) = dom();
        assert!(m(&d, "body", body) && m(&d, ".theme-dark", body) && m(&d, ":root", body));
        assert!(m(&d, ".theme-dark .workspace-leaf", leaf));
        assert!(m(&d, "body > .workspace > .workspace-leaf.mod-active", leaf));
        assert!(!m(&d, "body > .workspace-leaf", leaf));
        assert!(m(&d, ".workspace-leaf[data-type=\"markdown\"] a.internal-link", a));
        assert!(m(&d, "[data-type^=mark][data-type$=down][data-type*=kd]", leaf));
        assert!(m(&d, "a[href~=Note]", a) && !m(&d, "a[href=Note]", a) && m(&d, "a[href]", a));
        assert!(m(&d, "h1 + p", p) && m(&d, "h1 ~ p", p) && !m(&d, "p + h1", h1));
        assert!(m(&d, "h1:first-child", h1) && m(&d, "p:last-child", p) && !m(&d, "h1:last-child", h1));
        assert!(m(&d, "a:not(.external-link)", a) && !m(&d, "a:not(.is-unresolved)", a));
        assert!(m(&d, ":is(h1, h2).cm-header", h1) && m(&d, "*", ws) && m(&d, "#title", h1));
        assert!(!m(&d, "a:hover", a));
        d.nodes[a].hover = true;
        assert!(m(&d, ".theme-dark a:hover", a));
    }

    #[test]
    fn specificity_order() {
        let s = |x: &str| parse_selector(x).unwrap().specificity;
        assert!(s("#a") > s(".a.b.c") && s(".a.b") > s(".a") && s(".a") > s("div p") && s("div p") > s("p"));
        assert_eq!(s(".a:hover"), s(".a.b"));
        assert_eq!(s(":where(.a) p"), s("p"));
        assert_eq!(s("*"), 0);
    }

    #[test]
    fn rejects_unsupported() {
        for bad in [".a::before", ".a:nth-child(2)", "..x", "a >", "[x", ""] {
            assert!(parse_selector(bad).is_err(), "{bad}");
        }
        assert_eq!(parse_selector_list(".a, .b > .c ,p").unwrap().len(), 3);
    }
}
