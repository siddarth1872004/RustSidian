use md_engine::parse;

const KITCHEN_SINK: &str = "---\ntitle: Demo\ntags: [a, b]\n---\n# Heading *one*\n\nPara with [[Note#Sec|alias]], ![[img.png]], #tag/sub, ==hi==, ~~no~~, `code`, $x$ and a footnote[^n]. ^para-1\n\n> [!tip]+ Title\n> - [x] done\n> - [ ] todo\n\n| a | b |\n|---|:-:|\n| 1 | **2** |\n\n```mermaid\ngraph TD; A-->B\n```\n\n1. one\n2. two\n   continued\n\n   second para\n\n[^n]: Footnote text\n\n%%\nhidden\n%%\n\n$$\nE = mc^2\n$$\n";

#[test]
fn kitchen_sink_tree() {
    let doc = parse(KITCHEN_SINK);
    insta::assert_snapshot!(doc.dump(KITCHEN_SINK));
}

#[test]
fn kitchen_sink_html() {
    let doc = parse(KITCHEN_SINK);
    insta::assert_snapshot!(md_engine::html::render(&doc, KITCHEN_SINK));
}
