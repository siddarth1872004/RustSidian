//! Property tests standing in for fuzzing on machines without cargo-fuzz: the CSS parser and
//! value parsers must never panic and must terminate on arbitrary input.
use proptest::prelude::*;
use theme_engine::{css, parse_color, parse_length, LengthCx, Theme};

const FRAG: &[&str] = &[
    ".a",
    "#b",
    "body",
    " ",
    ">",
    "+",
    "~",
    ",",
    "{",
    "}",
    ":",
    ";",
    "color",
    "red",
    "var(--x",
    "var(--x)",
    ")",
    "(",
    "--x",
    "calc(",
    "1px",
    "@media",
    "(prefers-color-scheme: dark)",
    "/*",
    "*/",
    "\"",
    "'",
    "[",
    "]",
    "=",
    ":hover",
    ":not(",
    "::before",
    "!important",
    "\\",
    "\n",
    "é",
    "@import",
    "url(",
    "100%",
    "*",
    "-",
    "/",
    "rgb(",
    "hsl(",
    "#fff",
    "0",
    ".5s",
];

proptest! {
    #![proptest_config(ProptestConfig { cases: 3000, ..ProptestConfig::default() })]

    #[test]
    fn css_parser_never_panics(parts in prop::collection::vec(prop::sample::select(FRAG), 0..80)) {
        let src = parts.concat();
        let sheet = css::parse(&src);
        let mut t = Theme::new();
        t.add_sheet("fuzz", &src);
        prop_assert!(sheet.rules.len() <= src.len());
    }

    #[test]
    fn arbitrary_text_never_panics(src in "\\PC{0,300}") {
        let _ = css::parse(&src);
        let _ = css::parse_declarations(&src);
        let _ = parse_color(&src);
        let _ = parse_length(&src, &LengthCx::default());
        let _ = theme_engine::selector::parse_selector_list(&src);
        let _ = theme_engine::value::parse_transitions(&src);
    }
}
