use md_engine::{html, parse};

fn h(md: &str) -> String {
    html::render(&parse(md), md)
}

macro_rules! cases {
    ($($name:ident: $md:expr => $html:expr;)*) => {
        $(#[test] fn $name() { assert_eq!(h($md), $html, "\nsource: {:?}", $md); })*
    };
}

cases! {
    paragraph: "hello\nworld\n" => "<p>hello\nworld</p>\n";
    hard_break: "a  \nb\\\nc" => "<p>a  <br>\nb<br>\nc</p>\n";
    atx: "# One\n## Two ##\n#notheading\n" => "<h1 data-heading=\"One\">One</h1>\n<h2 data-heading=\"Two\">Two</h2>\n<p><a class=\"tag\" href=\"#notheading\">#notheading</a></p>\n";
    setext: "Title\n=====\nSub\n---\n" => "<h1 data-heading=\"Title\">Title</h1>\n<h2 data-heading=\"Sub\">Sub</h2>\n";
    emphasis: "*a* **b** ***c*** _d_ __e__ snake_case_word 2*3*4" => "<p><em>a</em> <strong>b</strong> <em><strong>c</strong></em> <em>d</em> <strong>e</strong> snake_case_word 2<em>3</em>4</p>\n";
    nested_emphasis: "**bold *and italic* text**" => "<p><strong>bold <em>and italic</em> text</strong></p>\n";
    unmatched: "a * b ** c_ _d" => "<p>a * b ** c_ _d</p>\n";
    strike_highlight: "~~gone~~ ==marked== a=b ~one~ ===x===" => "<p><del>gone</del> <mark>marked</mark> a=b <del>one</del> =<mark>x</mark>=</p>\n";
    code_span: "`a*b*` ``c ` d`` `unclosed" => "<p><code>a*b*</code> <code>c ` d</code> `unclosed</p>\n";
    fenced: "```rust title\nfn main() {}\n```\n" => "<pre><code class=\"language-rust\">fn main() {}\n</code></pre>\n";
    fenced_unclosed: "~~~\na\nb" => "<pre><code>a\nb</code></pre>\n";
    indented_code: "    code\n\n    more\ntext" => "<pre><code>code\n\nmore\n</code></pre>\n<p>text</p>\n";
    quote: "> a\n> b\nlazy\n\n> c" => "<blockquote>\n<p>a\nb\nlazy</p>\n</blockquote>\n<blockquote>\n<p>c</p>\n</blockquote>\n";
    callout: "> [!warning]- Be *careful*\n> body\n" => "<div class=\"callout is-collapsed\" data-callout=\"warning\" data-callout-fold=\"-\">\n<div class=\"callout-title\">Be <em>careful</em></div>\n<div class=\"callout-content\">\n<p>body</p>\n</div>\n</div>\n";
    callout_default_title: "> [!NOTE]\n> x" => "<div class=\"callout\" data-callout=\"note\">\n<div class=\"callout-title\">Note</div>\n<div class=\"callout-content\">\n<p>x</p>\n</div>\n</div>\n";
    bullets: "- a\n- b\n  - c\n- d\n" => "<ul>\n<li>a</li>\n<li>b\n<ul>\n<li>c</li>\n</ul></li>\n<li>d</li>\n</ul>\n";
    ordered: "3. a\n4. b\n" => "<ol start=\"3\">\n<li>a</li>\n<li>b</li>\n</ol>\n";
    loose_list: "- a\n\n- b\n" => "<ul>\n<li><p>a</p>\n</li>\n<li><p>b</p>\n</li>\n</ul>\n";
    tasks: "- [ ] todo\n- [x] done\n- [/] half\n" => "<ul class=\"contains-task-list\">\n<li class=\"task-list-item\" data-task=\" \"><input type=\"checkbox\" class=\"task-list-item-checkbox\"> todo</li>\n<li class=\"task-list-item is-checked\" data-task=\"x\"><input type=\"checkbox\" class=\"task-list-item-checkbox\" checked> done</li>\n<li class=\"task-list-item is-checked\" data-task=\"/\"><input type=\"checkbox\" class=\"task-list-item-checkbox\" checked> half</li>\n</ul>\n";
    list_then_para: "- a\n\npara" => "<ul>\n<li>a</li>\n</ul>\n<p>para</p>\n";
    different_bullets: "- a\n* b" => "<ul>\n<li>a</li>\n</ul>\n<ul>\n<li>b</li>\n</ul>\n";
    table: "| a | b |\n|:--|--:|\n| 1 | [[x\\|y]] |\n" => "<table>\n<thead>\n<tr><th style=\"text-align:left\">a</th><th style=\"text-align:right\">b</th></tr>\n</thead>\n<tbody>\n<tr><td style=\"text-align:left\">1</td><td style=\"text-align:right\"><a class=\"internal-link\" data-href=\"x\" href=\"x\">y</a></td></tr>\n</tbody>\n</table>\n";
    hr: "a\n\n---\n***\n" => "<p>a</p>\n<hr>\n<hr>\n";
    wikilinks: "[[Note]] [[Dir/Note#Heading|alias]] [[#local]] [[Note#^blk]]" => "<p><a class=\"internal-link\" data-href=\"Note\" href=\"Note\">Note</a> <a class=\"internal-link\" data-href=\"Dir/Note#Heading\" href=\"Dir/Note#Heading\">alias</a> <a class=\"internal-link\" data-href=\"#local\" href=\"#local\">local</a> <a class=\"internal-link\" data-href=\"Note#^blk\" href=\"Note#^blk\">Note &gt; ^blk</a></p>\n";
    embeds: "![[img.png|300]] ![[Note#Sec]]" => "<p><span class=\"internal-embed\" src=\"img.png\" alt=\"300\"></span> <span class=\"internal-embed\" src=\"Note#Sec\"></span></p>\n";
    links: "[a](http://x.y \"t\") [b](<my file.md>) ![alt *x*](i.png) [ref][r] [nope]\n\n[r]: /dest\n" => "<p><a href=\"http://x.y\" title=\"t\" class=\"external-link\">a</a> <a href=\"my file.md\">b</a> <img src=\"i.png\" alt=\"alt x\"> <a href=\"/dest\">ref</a> [nope]</p>\n";
    autolinks: "<https://a.b> https://c.d/e(f). www.x.org, <me@x.org>" => "<p><a href=\"https://a.b\" class=\"external-link\">https://a.b</a> <a href=\"https://c.d/e(f)\" class=\"external-link\">https://c.d/e(f)</a>. <a href=\"http://www.x.org\" class=\"external-link\">www.x.org</a>, <a href=\"mailto:me@x.org\" class=\"external-link\">me@x.org</a></p>\n";
    tags: "#tag #nested/tag #123 #a1 x#no (#paren) #日本語" => "<p><a class=\"tag\" href=\"#tag\">#tag</a> <a class=\"tag\" href=\"#nested/tag\">#nested/tag</a> #123 <a class=\"tag\" href=\"#a1\">#a1</a> x#no (<a class=\"tag\" href=\"#paren\">#paren</a>) <a class=\"tag\" href=\"#日本語\">#日本語</a></p>\n";
    comments: "a %%hidden%% b\n\n%%\nblock\n%%\n\nc" => "<p>a  b</p>\n<p>c</p>\n";
    math: "$x^2$ costs $5 and $10 $$y$$\n\n$$\n\\int x\n$$\n" => "<p><span class=\"math math-inline\">x^2</span> costs $5 and $10 <span class=\"math math-block\">y</span></p>\n<div class=\"math math-block\">\\int x</div>\n";
    footnotes: "a[^1] b^[inline *n*]\n\n[^1]: The note\n    more\n" => "<p>a<sup class=\"footnote-ref\"><a href=\"#fn-1\">[1]</a></sup> b<sup class=\"footnote-ref\"><a href=\"#fn-inline-2\">[inline-2]</a></sup></p>\n<section class=\"footnotes\">\n<hr>\n<ol>\n<li id=\"fn-1\"><p>The note\nmore</p></li>\n<li id=\"fn-inline-2\">inline <em>n</em></li>\n</ol>\n</section>\n";
    block_id: "para text ^abc-1\n\n- item ^i1\n" => "<p>para text</p>\n<ul>\n<li>item</li>\n</ul>\n";
    html_passthrough: "<div class=\"x\">\n*raw*\n</div>\n\na <b>bold</b> <!-- c -->" => "<div class=\"x\">\n*raw*\n</div>\n<p>a <b>bold</b> <!-- c --></p>\n";
    escapes: "\\*not\\* \\[\\[x]] &amp; &copy; &#65; a < b" => "<p>*not* [[x]] &amp; © A a &lt; b</p>\n";
    front_matter: "---\ntitle: x\n---\n# H\n" => "<h1 data-heading=\"H\">H</h1>\n";
    not_front_matter: "text\n\n---\na: b\n---\n" => "<p>text</p>\n<hr>\n<h2 data-heading=\"a: b\">a: b</h2>\n";
    crlf: "# A\r\n\r\ntext\r\n- x\r\n" => "<h1 data-heading=\"A\">A</h1>\n<p>text</p>\n<ul>\n<li>x</li>\n</ul>\n";
    heading_in_quote_list: "> # H\n> - a\n>   b\n" => "<blockquote>\n<h1 data-heading=\"H\">H</h1>\n<ul>\n<li>a\nb</li>\n</ul>\n</blockquote>\n";
    code_in_list: "- a\n  ```js\n  x\n  ```\n" => "<ul>\n<li>a\n<pre><code class=\"language-js\">x\n</code></pre></li>\n</ul>\n";
    empty: "" => "";
    only_blank: "\n\n  \n" => "";
}

#[test]
fn spans_tile_the_document() {
    let md = "---\na: 1\n---\n\n# H\n\npara\n\n\n- a\n- b\n\n```\ncode\n```\n";
    let doc = parse(md);
    let mut pos = 0;
    for b in &doc.blocks {
        assert_eq!(b.start, pos);
        pos = b.end();
    }
    assert_eq!(pos, md.len());
}

#[test]
fn gfm_only_options() {
    let md = "[[x]] #t ==h== %%c%%";
    let doc = md_engine::Document::parse(md, md_engine::Options::gfm());
    assert_eq!(html::render(&doc, md), "<p>[[x]] #t ==h== %%c%%</p>\n");
}
