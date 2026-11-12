use crate::scan::{is_ident, push, sw};
use crate::{Class, Token};

pub fn html(src: &str, base: usize, out: &mut Vec<Token>) {
    let b = src.as_bytes();
    let n = b.len();
    let mut i = 0;
    while i < n {
        if sw(src, i, "<!--") {
            let e = src[i..].find("-->").map_or(n, |x| i + x + 3);
            push(out, base, i, e, Class::Comment);
            i = e;
        } else if b[i] == b'<'
            && b.get(i + 1).is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, b'/' | b'!' | b'?'))
        {
            let mut j = i + 1;
            while j < n && matches!(b[j], b'/' | b'!' | b'?') {
                j += 1;
            }
            let ns = j;
            while j < n && (is_ident(b[j]) || matches!(b[j], b'-' | b':' | b'.')) {
                j += 1;
            }
            push(out, base, i, j, Class::Tag);
            let name = src[ns..j].to_ascii_lowercase();
            while j < n && b[j] != b'>' {
                let c = b[j];
                if c == b'"' || c == b'\'' {
                    let e = src[j + 1..].find(c as char).map_or(n, |x| j + 2 + x);
                    push(out, base, j, e, Class::String);
                    j = e;
                } else if is_ident(c) {
                    let s = j;
                    while j < n && (is_ident(b[j]) || matches!(b[j], b'-' | b':' | b'.')) {
                        j += 1;
                    }
                    push(out, base, s, j, Class::Property);
                } else if c == b'/' {
                    push(out, base, j, j + 1, Class::Tag);
                    j += 1;
                } else {
                    j += 1;
                }
            }
            if j < n {
                push(out, base, j, j + 1, Class::Tag);
                j += 1;
            }
            i = j;
            if name == "script" || name == "style" {
                let close = if name == "script" { "</script" } else { "</style" };
                let e = src[i..].to_ascii_lowercase().find(close).map_or(n, |x| i + x);
                crate::run(if name == "script" { "js" } else { "css" }, &src[i..e], base + i, out);
                i = e;
            }
        } else if b[i] == b'&' {
            let e = src[i..]
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '&' || c == '#'))
                .map_or(n, |x| i + x);
            if b.get(e) == Some(&b';') {
                push(out, base, i, e + 1, Class::Important);
                i = e + 1;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
}

pub fn css(src: &str, base: usize, out: &mut Vec<Token>) {
    let b = src.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut depth = 0usize;
    let mut decl = false;
    let mut in_value = false;
    let mut stmt = true;
    while i < n {
        let c = b[i];
        if stmt && !c.is_ascii_whitespace() && !sw(src, i, "/*") {
            stmt = false;
            in_value = false;
            let term = src[i..].find([';', '{', '}']).map(|x| b[i + x]);
            decl = depth > 0 && term != Some(b'{');
        }
        if sw(src, i, "/*") {
            let e = src[i..].find("*/").map_or(n, |x| i + x + 2);
            push(out, base, i, e, Class::Comment);
            i = e;
        } else if c == b'"' || c == b'\'' {
            let mut j = i + 1;
            while j < n && b[j] != c && b[j] != b'\n' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            let e = (j + 1).min(n);
            push(out, base, i, e, Class::String);
            i = e;
        } else if c == b'{' {
            depth += 1;
            stmt = true;
            i += 1;
        } else if c == b'}' {
            depth = depth.saturating_sub(1);
            stmt = true;
            i += 1;
        } else if c == b';' {
            stmt = true;
            i += 1;
        } else if c == b'@' {
            let mut e = i + 1;
            while e < n && (is_ident(b[e]) || b[e] == b'-') {
                e += 1;
            }
            push(out, base, i, e, Class::Keyword);
            i = e;
        } else if decl && !in_value {
            if c == b':' {
                in_value = true;
                i += 1;
            } else if is_ident(c) || c == b'-' {
                let mut e = i + 1;
                while e < n && (is_ident(b[e]) || b[e] == b'-') {
                    e += 1;
                }
                push(out, base, i, e, Class::Property);
                i = e;
            } else {
                i += 1;
            }
        } else if decl {
            if c == b'#' {
                let mut e = i + 1;
                while e < n && b[e].is_ascii_hexdigit() {
                    e += 1;
                }
                push(out, base, i, e, Class::Constant);
                i = e;
            } else if c == b'!' {
                let e = src[i..].find(';').map_or(n, |x| i + x);
                push(out, base, i, e, Class::Important);
                i = e;
            } else if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
                let mut e = i + 1;
                while e < n && (b[e].is_ascii_alphanumeric() || b[e] == b'.' || b[e] == b'%') {
                    e += 1;
                }
                push(out, base, i, e, Class::Number);
                i = e;
            } else if is_ident(c) || c == b'-' {
                let mut e = i + 1;
                while e < n && (is_ident(b[e]) || b[e] == b'-') {
                    e += 1;
                }
                if b.get(e) == Some(&b'(') {
                    push(out, base, i, e, Class::Function);
                }
                i = e;
            } else {
                i += 1;
            }
        } else if c == b'.' || c == b'#' || c == b':' || is_ident(c) {
            let mut e = i + 1;
            while e < n && (is_ident(b[e]) || b[e] == b'-' || (c == b':' && b[e] == b':')) {
                e += 1;
            }
            let class = if c == b':' { Class::Function } else { Class::Tag };
            push(out, base, i, e, class);
            i = e;
        } else {
            i += 1;
        }
    }
}

pub fn diff(src: &str, base: usize, out: &mut Vec<Token>) {
    let mut pos = 0;
    for line in src.split_inclusive('\n') {
        let t = line.trim_end_matches(['\n', '\r']);
        let class = if t.starts_with("+++")
            || t.starts_with("---")
            || t.starts_with("diff ")
            || t.starts_with("index ")
        {
            Some(Class::Important)
        } else if t.starts_with("@@") {
            Some(Class::Tag)
        } else if t.starts_with('+') || t.starts_with('>') {
            Some(Class::Inserted)
        } else if t.starts_with('-') || t.starts_with('<') {
            Some(Class::Deleted)
        } else {
            None
        };
        if let Some(c) = class {
            push(out, base, pos, pos + t.len(), c);
        }
        pos += line.len();
    }
}

pub fn markdown(src: &str, base: usize, out: &mut Vec<Token>) {
    let mut pos = 0;
    let mut fence = false;
    for line in src.split_inclusive('\n') {
        let t = line.trim_end_matches(['\n', '\r']);
        let s = t.trim_start();
        let ind = t.len() - s.len();
        let (a, e) = (pos, pos + t.len());
        if s.starts_with("```") || s.starts_with("~~~") {
            fence = !fence;
            push(out, base, a, e, Class::Comment);
        } else if fence {
            push(out, base, a, e, Class::String);
        } else if s.starts_with('#') {
            push(out, base, a, e, Class::Keyword);
        } else if s.starts_with('>') {
            push(out, base, a, e, Class::Comment);
        } else {
            let mut off = ind;
            if s.starts_with("- ") || s.starts_with("* ") || s.starts_with("+ ") {
                push(out, base, a + off, a + off + 1, Class::Operator);
                off += 2;
            }
            inline(&t[off..], base + a + off, out);
        }
        pos += line.len();
    }
}

fn inline(t: &str, base: usize, out: &mut Vec<Token>) {
    let b = t.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'`' => {
                let e = t[i + 1..].find('`').map_or(b.len(), |x| i + x + 2);
                push(out, base, i, e, Class::String);
                i = e;
            }
            b'[' => {
                let close = t[i..].find("](").and_then(|x| t[i + x..].find(')').map(|y| i + x + y + 1));
                if let Some(e) = close {
                    let mid = t[i..].find("](").map_or(e, |x| i + x);
                    push(out, base, i, mid + 1, Class::Function);
                    push(out, base, mid + 1, e, Class::String);
                    i = e;
                } else {
                    i += 1;
                }
            }
            b'*' | b'_' if b.get(i + 1) == Some(&b[i]) => {
                let m = &t[i..i + 2];
                if let Some(x) = t[i + 2..].find(m) {
                    push(out, base, i, i + 2 + x + 2, Class::Important);
                    i += 2 + x + 2;
                } else {
                    i += 2;
                }
            }
            _ => i += 1,
        }
    }
}
