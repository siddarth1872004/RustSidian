//! Query evaluation and the in-memory search corpus.

use crate::query::{Cmp, Expr, Op, PropValue};
use md_engine::frontmatter::Value;
use memchr::memmem;
use rayon::prelude::*;
use regex::Regex;
use std::path::Path;
use vault_core::meta::NoteMeta;
use vault_core::{path, FileId, Vault};

/// Lowercase without changing byte length, so offsets in the lowercased text are valid in the
/// original. Characters whose lowercase form has a different UTF-8 length are left as they are.
pub fn lower_same_len(s: &str) -> String {
    if s.is_ascii() {
        return s.to_ascii_lowercase();
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let mut l = c.to_lowercase();
        match (l.next(), l.next()) {
            (Some(x), None) if x.len_utf8() == c.len_utf8() => out.push(x),
            _ => out.push(c),
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Match {
    pub start: u32,
    pub end: u32,
}

pub struct NoteView<'a> {
    pub path: &'a str,
    /// Original text. May be the lowercased text when the query has no case-sensitive part.
    pub text: &'a str,
    pub lower: &'a str,
    pub meta: Option<&'a NoteMeta>,
}

enum C {
    And(Vec<C>),
    Or(Vec<C>),
    Not(Box<C>),
    Term { raw: String, lower: String, finders: Box<(memmem::Finder<'static>, memmem::Finder<'static>)> },
    Regex { exact: Regex, fold: Regex },
    Op(Op, Box<C>),
    Property(String, CProp),
}

enum CProp {
    Any,
    Null,
    Compare(Cmp, f64),
    Expr(Box<C>),
}

pub struct Query {
    root: C,
    needs_original: bool,
    meta_parts: u8,
}

fn meta_parts(c: &C) -> u8 {
    use vault_core::cache::parts::*;
    match c {
        C::And(v) | C::Or(v) => v.iter().fold(0, |a, c| a | meta_parts(c)),
        C::Not(x) => meta_parts(x),
        C::Term { .. } | C::Regex { .. } => 0,
        C::Op(op, x) => {
            meta_parts(x)
                | match op {
                    Op::Tag => TAGS,
                    Op::Block => SECTIONS,
                    Op::Section => HEADINGS,
                    Op::Task | Op::TaskTodo | Op::TaskDone => LIST_ITEMS,
                    _ => 0,
                }
        }
        C::Property(_, want) => {
            FRONTMATTER
                | match want {
                    CProp::Expr(x) => meta_parts(x),
                    _ => 0,
                }
        }
    }
}

const MAX_SPANS: usize = 200;

struct Hay<'a> {
    text: &'a str,
    lower: &'a str,
    /// Offset of this haystack in the note content, if it is part of the content.
    base: Option<u32>,
    /// The whole note: bare terms also match the file name.
    whole: bool,
}

impl Query {
    pub fn parse(q: &str) -> Result<Query, crate::query::ParseError> {
        Query::compile(&crate::query::parse(q)?)
    }

    pub fn compile(e: &Expr) -> Result<Query, crate::query::ParseError> {
        fn has_case(e: &Expr) -> bool {
            match e {
                Expr::And(v) | Expr::Or(v) => v.iter().any(has_case),
                Expr::Not(x) => has_case(x),
                Expr::Op(Op::MatchCase, _) => true,
                Expr::Op(_, x) => has_case(x),
                Expr::Property(_, PropValue::Expr(x)) => has_case(x),
                _ => false,
            }
        }
        fn go(e: &Expr) -> Result<C, crate::query::ParseError> {
            Ok(match e {
                Expr::And(v) => C::And(v.iter().map(go).collect::<Result<_, _>>()?),
                Expr::Or(v) => C::Or(v.iter().map(go).collect::<Result<_, _>>()?),
                Expr::Not(x) => C::Not(Box::new(go(x)?)),
                Expr::Term(t) => {
                    let lower = lower_same_len(t);
                    let finders = Box::new((
                        memmem::Finder::new(t.as_bytes()).into_owned(),
                        memmem::Finder::new(lower.as_bytes()).into_owned(),
                    ));
                    C::Term { raw: t.clone(), lower, finders }
                }
                Expr::Regex(p) => {
                    let err = |e: regex::Error| crate::query::ParseError {
                        offset: 0,
                        message: format!("invalid regex: {e}"),
                    };
                    let p = &p.replace("\\/", "/");
                    C::Regex {
                        exact: Regex::new(p).map_err(err)?,
                        fold: Regex::new(&format!("(?i){p}")).map_err(err)?,
                    }
                }
                Expr::Op(op, x) => C::Op(*op, Box::new(go(x)?)),
                Expr::Property(k, v) => C::Property(
                    k.clone(),
                    match v {
                        PropValue::Any => CProp::Any,
                        PropValue::Null => CProp::Null,
                        PropValue::Compare(c, n) => CProp::Compare(*c, *n),
                        PropValue::Expr(x) => CProp::Expr(Box::new(go(x)?)),
                    },
                ),
            })
        }
        let root = go(e)?;
        let meta_parts = meta_parts(&root);
        Ok(Query { root, needs_original: has_case(e), meta_parts })
    }

    /// The `vault_core::cache::parts` of note metadata that evaluation reads (0 if none).
    pub fn meta_parts(&self) -> u8 {
        self.meta_parts
    }

    /// Whether evaluation needs the original (not lowercased) text of a note.
    pub fn needs_original(&self) -> bool {
        self.needs_original
    }

    /// Evaluate against one note. Content match spans are appended to `out`.
    pub fn matches(&self, note: &NoteView<'_>, out: &mut Vec<Match>) -> bool {
        let hay = Hay { text: note.text, lower: note.lower, base: Some(0), whole: true };
        eval(&self.root, note, &hay, false, out)
    }
}

fn find_all(
    hay: &Hay<'_>,
    raw: &str,
    finders: &(memmem::Finder<'_>, memmem::Finder<'_>),
    case: bool,
    out: &mut Vec<Match>,
) -> bool {
    if raw.is_empty() {
        return true;
    }
    let (h, f) = if case { (hay.text, &finders.0) } else { (hay.lower, &finders.1) };
    let Some(base) = hay.base else { return f.find(h.as_bytes()).is_some() };
    let mut any = false;
    for p in f.find_iter(h.as_bytes()) {
        any = true;
        if out.len() >= MAX_SPANS {
            break;
        }
        out.push(Match { start: base + p as u32, end: base + (p + raw.len()) as u32 });
    }
    any
}

fn sub<'a>(hay: &Hay<'a>, start: usize, end: usize) -> Hay<'a> {
    let (s, e) = (start.min(hay.text.len()), end.min(hay.text.len()));
    let ok = s <= e && hay.text.is_char_boundary(s) && hay.text.is_char_boundary(e);
    let (s, e) = if ok { (s, e) } else { (0, 0) };
    Hay { text: &hay.text[s..e], lower: &hay.lower[s..e], base: hay.base.map(|b| b + s as u32), whole: false }
}

fn plain<'a>(s: &'a str, lower: &'a str) -> Hay<'a> {
    Hay { text: s, lower, base: None, whole: false }
}

/// Runs `inner` on each sub-range; true if any matches. Spans are kept only for matching ranges.
fn any_range(
    inner: &C,
    note: &NoteView<'_>,
    hay: &Hay<'_>,
    case: bool,
    out: &mut Vec<Match>,
    ranges: impl Iterator<Item = (usize, usize)>,
) -> bool {
    let mut any = false;
    let mut scratch = Vec::new();
    for (s, e) in ranges {
        scratch.clear();
        if eval(inner, note, &sub(hay, s, e), case, &mut scratch) {
            any = true;
            out.extend(scratch.drain(..).take(MAX_SPANS.saturating_sub(out.len())));
        }
    }
    any
}

/// A literal every match of `c` must contain (the longest term of a conjunction).
fn required_literal(c: &C) -> Option<(&str, &str)> {
    match c {
        C::Term { raw, lower, .. } if !raw.is_empty() => Some((raw, lower)),
        C::And(parts) => parts.iter().filter_map(required_literal).max_by_key(|(r, _)| r.len()),
        _ => None,
    }
}

fn value_strings(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Null => {}
        Value::Bool(b) => out.push(b.to_string()),
        Value::Number(n) => out.push(md_engine::frontmatter::fmt_number(*n)),
        Value::String(s) => out.push(s.clone()),
        Value::List(items) => items.iter().for_each(|i| value_strings(i, out)),
        Value::Map(items) => items.iter().for_each(|(_, i)| value_strings(i, out)),
    }
}

fn eval(c: &C, note: &NoteView<'_>, hay: &Hay<'_>, case: bool, out: &mut Vec<Match>) -> bool {
    match c {
        C::And(parts) => parts.iter().all(|p| eval(p, note, hay, case, out)),
        C::Or(parts) => {
            let mut any = false;
            for p in parts {
                any |= eval(p, note, hay, case, out);
            }
            any
        }
        C::Not(x) => !eval(x, note, hay, case, &mut Vec::new()),
        C::Term { raw, lower, finders } => {
            let in_content = find_all(hay, raw, finders, case, out);
            if in_content || !hay.whole {
                return in_content;
            }
            let name = path::file_name(note.path);
            if case {
                name.contains(raw.as_str())
            } else {
                lower_same_len(name).contains(lower.as_str())
            }
        }
        C::Regex { exact, fold } => {
            let re = if case { exact } else { fold };
            let Some(base) = hay.base else { return re.is_match(hay.text) };
            let mut any = false;
            for m in re.find_iter(hay.text) {
                any = true;
                if out.len() >= MAX_SPANS {
                    break;
                }
                out.push(Match { start: base + m.start() as u32, end: base + m.end() as u32 });
            }
            any || (hay.whole && re.is_match(path::file_name(note.path)))
        }
        C::Op(op, inner) => match op {
            Op::File | Op::Path => {
                let s = if *op == Op::File { path::file_name(note.path) } else { note.path };
                let lower = lower_same_len(s);
                eval(inner, note, &plain(s, &lower), case, &mut Vec::new())
            }
            Op::Content => eval(inner, note, &Hay { whole: false, ..sub(hay, 0, hay.text.len()) }, case, out),
            Op::Tag => {
                let Some(meta) = note.meta else { return false };
                match &**inner {
                    C::Term { raw, .. } => {
                        let want = raw.trim_start_matches('#');
                        want.is_empty() && !meta.tags.is_empty() || !want.is_empty() && meta.has_tag(want)
                    }
                    other => meta.tags.iter().any(|t| {
                        let tag = format!("#{}", t.tag);
                        let lower = lower_same_len(&tag);
                        eval(other, note, &plain(&tag, &lower), case, &mut Vec::new())
                    }),
                }
            }
            Op::Line => {
                let text = hay.text;
                // If the inner query requires a literal, only lines containing it can match.
                if let Some((raw, lower)) = required_literal(inner) {
                    let (h, n) = if case { (hay.text, raw) } else { (hay.lower, lower) };
                    let mut last_end = 0;
                    let mut ranges = Vec::new();
                    for p in memmem::find_iter(h.as_bytes(), n.as_bytes()) {
                        if p < last_end {
                            continue;
                        }
                        let s = text[..p].rfind('\n').map_or(0, |x| x + 1);
                        let e = text[p..].find('\n').map_or(text.len(), |x| p + x);
                        last_end = e;
                        ranges.push((s, e));
                    }
                    return any_range(inner, note, hay, case, out, ranges.into_iter());
                }
                let mut start = 0;
                let lines = text.split('\n').map(move |l| {
                    let r = (start, start + l.len());
                    start += l.len() + 1;
                    r
                });
                any_range(inner, note, hay, case, out, lines)
            }
            Op::Block => match note.meta {
                Some(m) => any_range(
                    inner,
                    note,
                    hay,
                    case,
                    out,
                    m.sections.iter().map(|s| (s.start as usize, s.end as usize)),
                ),
                None => eval(inner, note, hay, case, out),
            },
            Op::Section => {
                let mut bounds: Vec<usize> = vec![0];
                if let Some(m) = note.meta {
                    bounds.extend(m.headings.iter().map(|h| h.start as usize).filter(|&s| s > 0));
                }
                bounds.push(hay.text.len());
                let ranges: Vec<(usize, usize)> = bounds.windows(2).map(|w| (w[0], w[1])).collect();
                any_range(inner, note, hay, case, out, ranges.into_iter())
            }
            Op::Task | Op::TaskTodo | Op::TaskDone => {
                let Some(m) = note.meta else { return false };
                let text = hay.text;
                let items = m.list_items.iter().filter(|i| match (op, i.task) {
                    (_, None) => false,
                    (Op::TaskTodo, Some(t)) => t == b' ',
                    (Op::TaskDone, Some(t)) => t != b' ',
                    _ => true,
                });
                let ranges = items.map(|i| {
                    let s = (i.start as usize).min(text.len());
                    let e = text.get(s..).and_then(|t| t.find('\n')).map_or(text.len(), |p| s + p);
                    (s, e)
                });
                any_range(inner, note, hay, case, out, ranges)
            }
            Op::MatchCase => eval(inner, note, hay, true, out),
            Op::IgnoreCase => eval(inner, note, hay, false, out),
        },
        C::Property(key, want) => {
            let Some(v) = note.meta.and_then(|m| m.property(key)) else { return false };
            match want {
                CProp::Any => true,
                CProp::Null => {
                    matches!(v, Value::Null)
                        || matches!(v, Value::String(s) if s.is_empty())
                        || matches!(v, Value::List(l) if l.is_empty())
                }
                CProp::Compare(cmp, n) => {
                    let mut vals = Vec::new();
                    value_strings(v, &mut vals);
                    vals.iter().filter_map(|s| s.trim().parse::<f64>().ok()).any(|x| match cmp {
                        Cmp::Lt => x < *n,
                        Cmp::Le => x <= *n,
                        Cmp::Gt => x > *n,
                        Cmp::Ge => x >= *n,
                    })
                }
                CProp::Expr(inner) => {
                    let mut vals = Vec::new();
                    value_strings(v, &mut vals);
                    vals.iter().any(|s| {
                        let lower = lower_same_len(s);
                        eval(inner, note, &plain(s, &lower), case, &mut Vec::new())
                    })
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Sort {
    #[default]
    Relevance,
    PathAsc,
    PathDesc,
    ModifiedNewest,
    ModifiedOldest,
}

/// Ranking weights; user-configurable.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RankWeights {
    pub per_match: f32,
    /// Added when the first match is near the top of the note.
    pub early_match: f32,
    /// Matches are divided by `1 + length_penalty * kilobytes`.
    pub length_penalty: f32,
}

impl Default for RankWeights {
    fn default() -> Self {
        RankWeights { per_match: 1.0, early_match: 2.0, length_penalty: 0.05 }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Hit {
    pub file: FileId,
    pub score: f32,
    pub matches: Vec<Match>,
}

/// Lowercased text of every note, held in memory while search is in use.
#[derive(Default)]
pub struct Corpus {
    lower: Vec<Option<Box<str>>>,
}

impl Corpus {
    /// Read and lowercase every note in the vault, in parallel.
    pub fn build(vault: &Vault, root: &Path) -> Corpus {
        let mut lower: Vec<Option<Box<str>>> = Vec::new();
        lower.resize_with(vault.id_bound(), || None);
        let notes: Vec<(FileId, std::path::PathBuf)> =
            vault.notes().map(|(id, f)| (id, root.join(&*f.path))).collect();
        let read: Vec<(FileId, Option<Box<str>>)> = notes
            .into_par_iter()
            .map(|(id, p)| (id, std::fs::read_to_string(p).ok().map(|t| lower_same_len(&t).into_boxed_str())))
            .collect();
        for (id, t) in read {
            lower[id as usize] = t;
        }
        Corpus { lower }
    }

    pub fn update(&mut self, id: FileId, text: Option<&str>) {
        if self.lower.len() <= id as usize {
            self.lower.resize_with(id as usize + 1, || None);
        }
        self.lower[id as usize] = text.map(|t| lower_same_len(t).into_boxed_str());
    }

    pub fn bytes(&self) -> usize {
        self.lower.iter().flatten().map(|t| t.len()).sum()
    }

    pub fn search(
        &self,
        vault: &Vault,
        root: &Path,
        q: &Query,
        sort: Sort,
        weights: RankWeights,
        limit: usize,
    ) -> Vec<Hit> {
        let mut hits: Vec<Hit> = (0..self.lower.len())
            .into_par_iter()
            .filter_map(|i| {
                let lower = self.lower[i].as_deref()?;
                let f = vault.entry(i as FileId)?;
                let original;
                let text = if q.needs_original() {
                    original = std::fs::read_to_string(root.join(&*f.path)).ok()?;
                    if original.len() != lower.len() {
                        return None;
                    }
                    original.as_str()
                } else {
                    lower
                };
                let mut matches = Vec::new();
                let meta = (q.meta_parts() != 0).then(|| f.meta_parts(q.meta_parts())).flatten();
                let note = NoteView { path: &f.path, text, lower, meta: meta.as_deref() };
                if !q.matches(&note, &mut matches) {
                    return None;
                }
                matches.sort_by_key(|m| (m.start, m.end));
                matches.dedup();
                let kb = lower.len() as f32 / 1024.0;
                let mut score =
                    matches.len() as f32 * weights.per_match / (1.0 + weights.length_penalty * kb);
                if matches.first().is_some_and(|m| m.start < 200) {
                    score += weights.early_match;
                }
                Some(Hit { file: i as FileId, score, matches })
            })
            .collect();
        let key = |h: &Hit| vault.entry(h.file).map(|f| (f.path.clone(), f.mtime));
        match sort {
            Sort::Relevance => hits.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.file.cmp(&b.file))),
            Sort::PathAsc => hits.sort_by_cached_key(|h| key(h).map(|k| k.0)),
            Sort::PathDesc => {
                hits.sort_by_cached_key(|h| key(h).map(|k| k.0));
                hits.reverse();
            }
            Sort::ModifiedNewest => hits.sort_by_cached_key(|h| std::cmp::Reverse(key(h).map(|k| k.1))),
            Sort::ModifiedOldest => hits.sort_by_cached_key(|h| key(h).map(|k| k.1)),
        }
        hits.truncate(limit);
        hits
    }
}

/// A one-line excerpt around a match, for result lists.
pub fn snippet(text: &str, m: Match, context: usize) -> (String, usize, usize) {
    let (s, e) = ((m.start as usize).min(text.len()), (m.end as usize).min(text.len()));
    let line_start = text[..s].rfind('\n').map_or(0, |p| p + 1);
    let line_end = text[e..].find('\n').map_or(text.len(), |p| e + p);
    let mut from = s.saturating_sub(context).max(line_start);
    while !text.is_char_boundary(from) {
        from += 1;
    }
    let mut to = (e + context).min(line_end);
    while !text.is_char_boundary(to) {
        to -= 1;
    }
    (text[from..to].to_string(), s - from, e - from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vault_core::meta::extract;

    const NOTE: &str = "---\nstatus: Draft\nduration: 3\naliases: [Alt]\nempty:\n---\n# Recipes\n\nMix the flour and Sugar.\nAdd eggs later. #baking/bread\n\n## Tasks\n\n- [ ] call Mom about flour\n- [x] buy eggs\n- plain item eggs\n\nHappyCat likes flour.\n";

    fn check(q: &str, path: &str) -> (bool, Vec<Match>) {
        let query = Query::parse(q).unwrap();
        let meta = extract(NOTE);
        let lower = lower_same_len(NOTE);
        let mut out = Vec::new();
        let ok = query.matches(&NoteView { path, text: NOTE, lower: &lower, meta: Some(&meta) }, &mut out);
        (ok, out)
    }
    fn yes(q: &str) {
        assert!(check(q, "Food/Baking Notes.md").0, "should match: {q}");
    }
    fn no(q: &str) {
        assert!(!check(q, "Food/Baking Notes.md").0, "should not match: {q}");
    }

    #[test]
    fn terms_and_boolean() {
        yes("flour");
        yes("FLOUR sugar");
        yes("\"flour and sugar\"");
        no("\"sugar and flour\"");
        yes("pasta OR eggs");
        no("pasta OR rice");
        yes("flour -pasta");
        no("flour -eggs");
        yes("(pasta OR flour) (rice OR sugar)");
        yes("baking");
        yes("");
    }

    #[test]
    fn file_path_content() {
        yes("file:baking");
        yes("file:\"Baking Notes\"");
        no("file:food");
        yes("path:food");
        yes("path:(food notes)");
        no("content:baking\\ notes");
        yes("content:flour");
        no("file:flour");
    }

    #[test]
    fn tags_and_properties() {
        yes("tag:#baking");
        yes("tag:baking/bread");
        no("tag:#bak");
        no("tag:#bread");
        yes("tag:");
        yes("[status]");
        yes("[status:draft]");
        yes("[status:Published OR Draft]");
        no("[status:Published]");
        yes("[duration:<5]");
        no("[duration:>5]");
        yes("[duration:>=3]");
        yes("[empty:null]");
        no("[status:null]");
        no("[missing]");
        yes("[aliases:alt]");
    }

    #[test]
    fn line_block_section() {
        yes("line:(mix flour)");
        no("line:(mix eggs)");
        yes("block:(mix eggs)");
        no("block:(mix mom)");
        yes("section:(mix eggs)");
        no("section:(mix mom)");
        yes("section:(mom eggs)");
    }

    #[test]
    fn tasks() {
        yes("task:eggs");
        yes("task:mom");
        yes("task-todo:mom");
        no("task-todo:eggs");
        yes("task-done:eggs");
        no("task-done:mom");
        no("task:plain");
        yes("task:");
    }

    #[test]
    fn case_and_regex() {
        yes("happycat");
        yes("match-case:HappyCat");
        no("match-case:happycat");
        yes("match-case:(HappyCat ignore-case:SUGAR)");
        yes("/fl[ao]ur/");
        yes("/MIX\\s+the/");
        no("match-case:/MIX the/");
        no("/^nothing$/");
        yes("line:(/^- \\[x\\]/ eggs)");
    }

    #[test]
    fn spans_point_at_matches() {
        let (ok, spans) = check("flour sugar", "x.md");
        assert!(ok);
        assert_eq!(spans.len(), 4);
        for m in &spans {
            let s = &NOTE[m.start as usize..m.end as usize];
            assert!(s.eq_ignore_ascii_case("flour") || s.eq_ignore_ascii_case("sugar"), "{s}");
        }
        let (text, s, e) = snippet(NOTE, spans[0], 8);
        assert_eq!(&text[s..e].to_lowercase(), "flour");
        assert!(!text.contains('\n'));
        // Line-scoped spans only cover the matching line.
        let (_, spans) = check("line:(mom flour)", "x.md");
        assert_eq!(spans.len(), 2);
    }

    #[test]
    fn lowercase_keeps_offsets() {
        for s in ["Ünïcode ÄÖ", "İstanbul ẞ", "plain ASCII", "日本語 Ωmega"] {
            assert_eq!(lower_same_len(s).len(), s.len());
        }
        assert_eq!(lower_same_len("ÄB"), "äb");
    }

    #[test]
    fn corpus_search_sorts_and_limits() {
        let d = tempfile::tempdir().unwrap();
        let files = [
            ("a.md", "apple apple apple"),
            ("b.md", "apple"),
            ("c.md", "banana"),
            ("sub/d.md", "Apple pie #fruit"),
        ];
        std::fs::create_dir_all(d.path().join("sub")).unwrap();
        for (p, t) in files {
            std::fs::write(d.path().join(p), t).unwrap();
        }
        let (store, _) = vault_core::VaultStore::open(d.path()).unwrap();
        let mut corpus = Corpus::build(&store.index, d.path());
        let run = |c: &Corpus, q: &str, sort| -> Vec<String> {
            c.search(&store.index, d.path(), &Query::parse(q).unwrap(), sort, RankWeights::default(), 10)
                .iter()
                .map(|h| store.index.entry(h.file).unwrap().path.to_string())
                .collect()
        };
        assert_eq!(run(&corpus, "apple", Sort::Relevance)[0], "a.md");
        assert_eq!(run(&corpus, "apple", Sort::PathDesc), vec!["sub/d.md", "b.md", "a.md"]);
        assert_eq!(run(&corpus, "match-case:Apple", Sort::PathAsc), vec!["sub/d.md"]);
        assert_eq!(run(&corpus, "tag:#fruit OR banana", Sort::PathAsc), vec!["c.md", "sub/d.md"]);
        let c = store.index.id("c.md").unwrap();
        corpus.update(c, Some("now an apple"));
        assert_eq!(run(&corpus, "apple", Sort::PathAsc).len(), 4);
        corpus.update(c, None);
        assert_eq!(run(&corpus, "apple", Sort::PathAsc).len(), 3);
    }
}
