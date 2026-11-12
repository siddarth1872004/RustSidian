use super::*;

fn classes<'a>(lang: &str, src: &'a str) -> Vec<(&'a str, Class)> {
    highlight(lang, src).iter().map(|t| (&src[t.start..t.end], t.class)).collect()
}

fn has(lang: &str, src: &str, text: &str, class: Class) -> bool {
    classes(lang, src).contains(&(text, class))
}

#[test]
fn rust_basics() {
    let s = "fn main() { let x: u32 = 42; // hi\n println!(\"a\"); }";
    assert!(has("rust", s, "fn", Class::Keyword));
    assert!(has("rs", s, "main", Class::Function));
    assert!(has("rust", s, "u32", Class::Type));
    assert!(has("rust", s, "42", Class::Number));
    assert!(has("rust", s, "// hi", Class::Comment));
    assert!(has("rust", s, "\"a\"", Class::String));
    assert!(has("rust", "let s = r#\"a\"b\"#;", "r#\"a\"b\"#", Class::String));
}

#[test]
fn python_and_triple() {
    let s = "def f(a):\n    \"\"\"doc\n    more\"\"\"\n    return None # c";
    assert!(has("python", s, "def", Class::Keyword));
    assert!(has("py", s, "f", Class::Function));
    assert!(has("py", s, "\"\"\"doc\n    more\"\"\"", Class::String));
    assert!(has("py", s, "# c", Class::Comment));
}

#[test]
fn json_keys_and_values() {
    let s = "{\"a\": [1, true, \"x\"]}";
    assert!(has("json", s, "\"a\"", Class::Property));
    assert!(has("json", s, "\"x\"", Class::String));
    assert!(has("json", s, "true", Class::Constant));
}

#[test]
fn yaml_toml() {
    assert!(has("yaml", "- name: foo # c\n", "name", Class::Property));
    assert!(has("yaml", "a: 1 # c\n", "# c", Class::Comment));
    assert!(has("toml", "[package]\nname = \"x\"\n", "[package]", Class::Tag));
    assert!(has("toml", "[package]\nname = \"x\"\n", "name", Class::Property));
}

#[test]
fn markup_languages() {
    let h = "<div class=\"a\">&amp;<script>let x = 1;</script></div>";
    assert!(has("html", h, "<div", Class::Tag));
    assert!(has("html", h, "class", Class::Property));
    assert!(has("html", h, "\"a\"", Class::String));
    assert!(has("html", h, "let", Class::Keyword));
    let c = "a.b > c { color: #fff; margin: 4px !important }";
    assert!(has("css", c, ".b", Class::Tag));
    assert!(has("css", c, "color", Class::Property));
    assert!(has("css", c, "#fff", Class::Constant));
    assert!(has("css", c, "4px", Class::Number));
}

#[test]
fn diff_sql_shell() {
    let d = "@@ -1 +1 @@\n-old\n+new\n ctx";
    assert!(has("diff", d, "-old", Class::Deleted));
    assert!(has("diff", d, "+new", Class::Inserted));
    assert!(has("sql", "SELECT a FROM t -- x", "SELECT", Class::Keyword));
    assert!(has("bash", "echo $HOME # c", "$HOME", Class::Variable));
}

#[test]
fn sorted_non_overlapping_and_boundaries() {
    let src = "fn é() { \"ü\\\" ö\" } /* ö */ // ñ\n<a b='é'>";
    for lang in ["rust", "js", "python", "html", "css", "yaml", "toml", "md", "diff", "unknown", "sql"] {
        let toks = highlight(lang, src);
        let mut last = 0;
        for t in &toks {
            assert!(t.start >= last && t.end > t.start, "{lang}");
            assert!(src.is_char_boundary(t.start) && src.is_char_boundary(t.end), "{lang}");
            last = t.end;
        }
    }
}

#[test]
fn lang_of_info_strips() {
    assert_eq!(lang_of_info("Rust {1,3}"), "rust");
    assert_eq!(lang_of_info("language-py"), "py");
}
