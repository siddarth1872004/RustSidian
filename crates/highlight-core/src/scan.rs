use crate::{Class, Token};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Keys {
    None,
    Yaml,
    Toml,
    Json,
}

pub struct Spec {
    pub line: &'static [&'static str],
    pub block: Option<(&'static str, &'static str)>,
    pub nested: bool,
    pub quotes: &'static str,
    pub triple: bool,
    pub rust_raw: bool,
    pub keywords: &'static [&'static str],
    pub types: &'static [&'static str],
    pub consts: &'static [&'static str],
    pub def_words: &'static [&'static str],
    pub ci: bool,
    pub dollar: bool,
    pub at_ident: bool,
    pub keys: Keys,
}

pub const PLAIN: Spec = Spec {
    line: &["//", "#"],
    block: Some(("/*", "*/")),
    nested: false,
    quotes: "\"'`",
    triple: false,
    rust_raw: false,
    keywords: &[],
    types: &[],
    consts: &[],
    def_words: &[],
    ci: false,
    dollar: false,
    at_ident: false,
    keys: Keys::None,
};

pub fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

pub fn push(out: &mut Vec<Token>, base: usize, a: usize, b: usize, class: Class) {
    if b > a {
        out.push(Token { start: base + a, end: base + b, class });
    }
}

pub fn sw(src: &str, i: usize, pat: &str) -> bool {
    src.as_bytes()[i..].starts_with(pat.as_bytes())
}

fn is_op(c: u8) -> bool {
    matches!(
        c,
        b'+' | b'-' | b'*' | b'/' | b'%' | b'=' | b'<' | b'>' | b'!' | b'&' | b'|' | b'^' | b'~' | b'?'
    )
}

pub fn generic(sp: &Spec, src: &str, base: usize, out: &mut Vec<Token>) {
    let b = src.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut bol = true;
    let mut after_def = false;
    while i < n {
        let c = b[i];
        if c == b'\n' {
            bol = true;
            i += 1;
            continue;
        }
        if c == b' ' || c == b'\t' || c == b'\r' {
            i += 1;
            continue;
        }
        let was_bol = std::mem::replace(&mut bol, false);
        if was_bol {
            if let Some(end) = line_key(sp, src, i, out, base) {
                i = end;
                continue;
            }
        }
        if sp.line.iter().any(|p| sw(src, i, *p)) && comment_ok(sp, b, i) {
            let e = src[i..].find('\n').map_or(n, |x| i + x);
            push(out, base, i, e, Class::Comment);
            i = e;
            continue;
        }
        if let Some((open, close)) = sp.block {
            if sw(src, i, open) {
                let e = block_end(sp, src, i, open, close);
                push(out, base, i, e, Class::Comment);
                i = e;
                continue;
            }
        }
        if sp.rust_raw && (c == b'r' || c == b'b') {
            if let Some(e) = raw_string(b, i) {
                push(out, base, i, e, Class::String);
                i = e;
                continue;
            }
        }
        if sp.quotes.as_bytes().contains(&c) {
            let e = string_end(sp, src, i);
            let class =
                if sp.keys == Keys::Json && next_is(b, e, b':') { Class::Property } else { Class::String };
            push(out, base, i, e, class);
            i = e;
            continue;
        }
        if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let mut e = i + 1;
            let hex = c == b'0' && matches!(b.get(i + 1), Some(b'x' | b'X'));
            while e < n {
                let d = b[e];
                if d.is_ascii_alphanumeric() || d == b'_' {
                    e += 1;
                } else if d == b'.' && b.get(e + 1).is_some_and(u8::is_ascii_digit) {
                    e += 1;
                } else if (d == b'+' || d == b'-') && !hex && matches!(b[e - 1], b'e' | b'E') {
                    e += 1;
                } else {
                    break;
                }
            }
            push(out, base, i, e, Class::Number);
            i = e;
            continue;
        }
        if (c == b'$' && sp.dollar) || (c == b'@' && sp.at_ident) {
            let mut e = i + 1;
            if sp.dollar && b.get(e) == Some(&b'{') {
                e = src[e..].find('}').map_or(n, |x| e + x + 1);
            } else {
                while e < n && is_ident(b[e]) {
                    e += 1;
                }
            }
            if e > i + 1 {
                push(out, base, i, e, if c == b'$' { Class::Variable } else { Class::Important });
                i = e;
                continue;
            }
        }
        if is_ident(c) {
            let mut e = i + 1;
            while e < n && is_ident(b[e]) {
                e += 1;
            }
            let word = &src[i..e];
            let lower;
            let w = if sp.ci {
                lower = word.to_ascii_lowercase();
                lower.as_str()
            } else {
                word
            };
            let class = if sp.keywords.contains(&w) {
                Some(Class::Keyword)
            } else if sp.consts.contains(&w) {
                Some(Class::Constant)
            } else if sp.types.contains(&w) {
                Some(Class::Type)
            } else if after_def || b.get(e) == Some(&b'(') {
                Some(Class::Function)
            } else if b[i].is_ascii_uppercase()
                && e - i > 1
                && word.bytes().any(|x| x.is_ascii_lowercase())
                && !sp.keywords.is_empty()
            {
                Some(Class::Type)
            } else {
                None
            };
            after_def = sp.def_words.contains(&w);
            if let Some(class) = class {
                push(out, base, i, e, class);
            }
            i = e;
            continue;
        }
        after_def = false;
        if is_op(c) {
            let mut e = i + 1;
            while e < n && is_op(b[e]) {
                e += 1;
            }
            push(out, base, i, e, Class::Operator);
            i = e;
            continue;
        }
        i += 1;
    }
}

fn comment_ok(sp: &Spec, b: &[u8], i: usize) -> bool {
    if sp.keys == Keys::Yaml {
        return i == 0 || matches!(b[i - 1], b' ' | b'\t' | b'\n');
    }
    true
}

fn next_is(b: &[u8], mut i: usize, want: u8) -> bool {
    while i < b.len() && matches!(b[i], b' ' | b'\t') {
        i += 1;
    }
    b.get(i) == Some(&want)
}

fn block_end(sp: &Spec, src: &str, i: usize, open: &str, close: &str) -> usize {
    let mut depth = 1;
    let mut j = i + open.len();
    while j < src.len() {
        if sp.nested && sw(src, j, open) {
            depth += 1;
            j += open.len();
        } else if sw(src, j, close) {
            depth -= 1;
            j += close.len();
            if depth == 0 {
                return j;
            }
        } else {
            j += 1;
        }
    }
    src.len()
}

fn raw_string(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i;
    if b[j] == b'b' {
        j += 1;
    }
    if b.get(j) != Some(&b'r') {
        return None;
    }
    j += 1;
    let h0 = j;
    while b.get(j) == Some(&b'#') {
        j += 1;
    }
    let hashes = j - h0;
    if b.get(j) != Some(&b'"') {
        return None;
    }
    j += 1;
    while j < b.len() {
        if b[j] == b'"' && (0..hashes).all(|k| b.get(j + 1 + k) == Some(&b'#')) {
            return Some(j + 1 + hashes);
        }
        j += 1;
    }
    Some(b.len())
}

fn string_end(sp: &Spec, src: &str, i: usize) -> usize {
    let b = src.as_bytes();
    let q = b[i];
    let mut j = i + 1;
    if sp.triple && b.get(i + 1) == Some(&q) && b.get(i + 2) == Some(&q) {
        j = i + 3;
        while j < b.len() {
            if b[j] == b'\\' {
                j += 2;
            } else if b[j] == q && b.get(j + 1) == Some(&q) && b.get(j + 2) == Some(&q) {
                return j + 3;
            } else {
                j += 1;
            }
        }
        return b.len();
    }
    let multi = q == b'`';
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            c if c == q => return j + 1,
            b'\n' if !multi => return j,
            _ => j += 1,
        }
    }
    b.len()
}

fn line_key(sp: &Spec, src: &str, i: usize, out: &mut Vec<Token>, base: usize) -> Option<usize> {
    let b = src.as_bytes();
    match sp.keys {
        Keys::Toml => {
            if b[i] == b'[' {
                let e = src[i..].find('\n').map_or(b.len(), |x| i + x);
                let line = src[i..e].trim_end();
                if line.ends_with(']') {
                    push(out, base, i, i + line.len(), Class::Tag);
                    return Some(i + line.len());
                }
                return None;
            }
            let mut e = i;
            while e < b.len() && (is_ident(b[e]) || b[e] == b'-' || b[e] == b'.') {
                e += 1;
            }
            (e > i && next_is(b, e, b'=')).then(|| {
                push(out, base, i, e, Class::Property);
                e
            })
        }
        Keys::Yaml => {
            let mut s = i;
            if b[s] == b'-' && b.get(s + 1) == Some(&b' ') {
                s += 2;
                while b.get(s) == Some(&b' ') {
                    s += 1;
                }
            }
            if s >= b.len() || b[s] == b'#' {
                return None;
            }
            let mut e = s;
            if matches!(b[s], b'"' | b'\'') {
                e = string_end(sp, src, s);
            } else {
                while e < b.len()
                    && !matches!(b[e], b'\n' | b'#')
                    && !(b[e] == b':' && matches!(b.get(e + 1), None | Some(b' ' | b'\n' | b'\r')))
                {
                    e += 1;
                }
            }
            (b.get(e) == Some(&b':')).then(|| {
                push(out, base, s, e, Class::Property);
                e
            })
        }
        _ => None,
    }
}
