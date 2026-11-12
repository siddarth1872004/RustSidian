//! Hand-written, dependency-free tokenizer for the languages that show up in notes.
//!
//! Tokens are non-overlapping and sorted; text between tokens is plain. Class names follow
//! Prism/CodeMirror (`.token.keyword`, `.cm-keyword`) and map to Obsidian's `--code-*` variables.

mod langs;
mod markup;
mod scan;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    Keyword,
    String,
    Comment,
    Number,
    Function,
    Type,
    Property,
    Tag,
    Operator,
    Important,
    Constant,
    Variable,
    Inserted,
    Deleted,
}

impl Class {
    /// The Prism class (`.token.<name>`); CodeMirror uses `.cm-<name>`.
    pub fn name(self) -> &'static str {
        match self {
            Class::Keyword => "keyword",
            Class::String => "string",
            Class::Comment => "comment",
            Class::Number => "number",
            Class::Function => "function",
            Class::Type => "class-name",
            Class::Property => "property",
            Class::Tag => "tag",
            Class::Operator => "operator",
            Class::Important => "important",
            Class::Constant => "boolean",
            Class::Variable => "variable",
            Class::Inserted => "inserted",
            Class::Deleted => "deleted",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub class: Class,
}

/// The language named by a fence info string, lowercased.
pub fn lang_of_info(info: &str) -> String {
    let w = info.split(|c: char| c.is_whitespace() || c == '{' || c == ':').next().unwrap_or("");
    w.trim_start_matches("language-").to_ascii_lowercase()
}

/// Tokens for `src` in language `lang` (any alias; unknown languages get a generic scan).
pub fn highlight(lang: &str, src: &str) -> Vec<Token> {
    let mut out = Vec::new();
    run(lang, src, 0, &mut out);
    out
}

/// Like [`highlight`], memoised on (language, text) so repeated frames do not re-tokenise.
pub fn highlight_cached(lang: &str, src: &str) -> std::rc::Rc<[Token]> {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::hash::{Hash, Hasher};
    thread_local! {
        static CACHE: RefCell<HashMap<u64, std::rc::Rc<[Token]>>> = RefCell::new(HashMap::new());
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    lang.hash(&mut h);
    src.hash(&mut h);
    let key = h.finish();
    CACHE.with(|c| {
        if let Some(t) = c.borrow().get(&key) {
            return t.clone();
        }
        let t: std::rc::Rc<[Token]> = highlight(lang, src).into();
        let mut c = c.borrow_mut();
        if c.len() >= 128 {
            c.clear();
        }
        c.insert(key, t.clone());
        t
    })
}

pub(crate) fn run(lang: &str, src: &str, base: usize, out: &mut Vec<Token>) {
    match lang {
        "html" | "xml" | "svg" | "xhtml" | "vue" => markup::html(src, base, out),
        "css" | "scss" | "less" => markup::css(src, base, out),
        "diff" | "patch" => markup::diff(src, base, out),
        "md" | "markdown" => markup::markdown(src, base, out),
        _ => scan::generic(langs::spec(lang), src, base, out),
    }
}

#[cfg(test)]
mod tests;
