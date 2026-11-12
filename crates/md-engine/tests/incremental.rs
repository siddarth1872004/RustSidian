use md_engine::{html, parse, Document, Edit, Options};
use proptest::prelude::*;

const FRAGMENTS: &[&str] = &[
    "\n",
    "\n\n",
    " ",
    "    ",
    "\t",
    "# ",
    "## ",
    "- ",
    "* ",
    "1. ",
    "2) ",
    "- [ ] ",
    "- [x] ",
    "> ",
    "> [!note]+ ",
    "```\n",
    "```rust\n",
    "~~~\n",
    "$$",
    "$",
    "%%",
    "---\n",
    "===\n",
    "***\n",
    "|",
    "| a | b |\n",
    "|---|:-:|\n",
    "[[",
    "]]",
    "![[",
    "[",
    "](",
    ")",
    "[^1]",
    "[^1]: ",
    "^[",
    "^id\n",
    "#tag",
    "*",
    "**",
    "_",
    "~~",
    "==",
    "`",
    "``",
    "\\",
    "<div>\n",
    "</div>",
    "<b>",
    "<!--",
    "-->",
    "&amp;",
    "https://a.b/c",
    "word",
    "two words",
    "x: 1\n",
    "é",
    "日本",
    "\r\n",
    "  \n",
    "a|b",
    "!",
];

fn doc_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(FRAGMENTS), 0..60).prop_map(|v| v.concat())
}

fn check_tiling(doc: &Document, text: &str) {
    let mut pos = 0;
    for b in &doc.blocks {
        assert_eq!(b.start, pos, "blocks must tile the text");
        assert!(b.len > 0);
        for (_, n) in b.tree.iter() {
            assert!(n.span.start <= n.span.end && n.span.end as usize <= b.len, "span out of block: {n:?}");
            assert!(text.is_char_boundary(b.start + n.span.start as usize));
            assert!(text.is_char_boundary(b.start + n.span.end as usize));
        }
        pos = b.end();
    }
    assert_eq!(pos, text.len());
}

fn floor(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2000, ..ProptestConfig::default() })]

    #[test]
    fn never_panics_and_tiles(text in doc_strategy()) {
        let doc = parse(&text);
        check_tiling(&doc, &text);
        let _ = html::render(&doc, &text);
        let _ = doc.dump(&text);
    }

    #[test]
    fn arbitrary_unicode_never_panics(text in "\\PC{0,200}") {
        let doc = parse(&text);
        check_tiling(&doc, &text);
        let _ = html::render(&doc, &text);
    }

    #[test]
    fn incremental_equals_full(base in doc_strategy(), at in 0usize..4000, del in 0usize..40, ins in doc_strategy()) {
        let start = floor(&base, at % (base.len() + 1));
        let old_end = floor(&base, (start + del).max(start));
        let ins = &ins[..floor(&ins, 30)];
        let mut text = base.clone();
        text.replace_range(start..old_end, ins);
        let mut doc = parse(&base);
        doc.edit(text.as_str(), Edit { start, old_end, new_end: start + ins.len() });
        let full = Document::parse(&text, Options::default());
        prop_assert_eq!(&doc, &full, "base={:?} text={:?}", base, text);
    }

    #[test]
    fn typing_sequence_equals_full(base in doc_strategy(), keys in prop::collection::vec((0usize..4000, prop::sample::select(FRAGMENTS)), 1..12)) {
        let mut text = base;
        let mut doc = parse(&text);
        for (at, k) in keys {
            let start = floor(&text, at % (text.len() + 1));
            text.insert_str(start, k);
            doc.edit(text.as_str(), Edit { start, old_end: start, new_end: start + k.len() });
        }
        prop_assert_eq!(&doc, &parse(&text));
    }
}

#[test]
fn large_document_edit_touches_few_blocks() {
    let para = "Some *text* with a [[link]] and #tag.\n\n";
    let mut text = para.repeat(20_000);
    let mut doc = parse(&text);
    let at = text.len() / 2 + 5;
    text.insert(at, 'x');
    let changed = doc.edit(text.as_str(), Edit { start: at, old_end: at, new_end: at + 1 });
    assert!(changed.len() <= 4, "reparsed {} blocks", changed.len());
    assert_eq!(doc, parse(&text));
}
