use super::*;

fn lay(src: &str, display: bool) -> Layout {
    render(src, 16.0, display, &mut Approx)
}

fn texts(l: &Layout) -> Vec<&str> {
    l.glyphs.iter().map(|g| g.text.as_str()).collect()
}

#[test]
fn symbols_and_greek() {
    let l = lay(r"\alpha + \beta \leq \infty", false);
    let t = texts(&l);
    for s in ["α", "β", "≤", "∞", "+"] {
        assert!(t.contains(&s), "{t:?}");
    }
}

#[test]
fn scripts_are_smaller_and_shifted() {
    let l = lay("x^2_i", false);
    let x = &l.glyphs[0];
    let sup = l.glyphs.iter().find(|g| g.text == "2").unwrap();
    let sub = l.glyphs.iter().find(|g| g.text == "i").unwrap();
    assert!(sup.size < x.size && sub.size < x.size);
    assert!(sup.y < x.y && sub.y > x.y);
    assert!(sup.x > x.x && (sup.x - sub.x).abs() < 0.01);
}

#[test]
fn fraction_has_rule_and_stacks() {
    let l = lay(r"\frac{a}{b}", true);
    assert_eq!(l.segs.len(), 1);
    let rule_y = l.segs[0].y0;
    let a = l.glyphs.iter().find(|g| g.text == "a").unwrap();
    let b = l.glyphs.iter().find(|g| g.text == "b").unwrap();
    assert!(a.y < rule_y && b.y > rule_y);
    assert!(l.height() > 16.0 * 1.5);
}

#[test]
fn sqrt_draws_radical() {
    let l = lay(r"\sqrt{x+1}", false);
    assert!(l.segs.len() >= 4);
    assert!(texts(&l).contains(&"x"));
}

#[test]
fn limits_in_display_are_stacked() {
    let l = lay(r"\sum_{i=0}^{n} i", true);
    let sum = l.glyphs.iter().find(|g| g.text == "∑").unwrap();
    let n = l.glyphs.iter().find(|g| g.text == "n").unwrap();
    let i0 = l.glyphs.iter().find(|g| g.text == "0").unwrap();
    assert!(n.y < sum.y && i0.y > sum.y);
    assert!(sum.size > 16.0);
    let inline = lay(r"\sum_{i=0}^{n} i", false);
    let sum = inline.glyphs.iter().find(|g| g.text == "∑").unwrap();
    let n = inline.glyphs.iter().find(|g| g.text == "n").unwrap();
    assert!(n.x > sum.x);
}

#[test]
fn integral_keeps_side_limits() {
    let l = lay(r"\int_0^1 f(x)\,dx", true);
    let int = l.glyphs.iter().find(|g| g.text == "∫").unwrap();
    let one = l.glyphs.iter().find(|g| g.text == "1").unwrap();
    assert!(one.x > int.x);
}

#[test]
fn left_right_scale_with_content() {
    let small = lay(r"\left( x \right)", false);
    assert!(small.segs.is_empty());
    let big = lay(r"\left( \frac{a}{b} \right)", true);
    assert!(big.segs.len() > 5);
}

#[test]
fn text_and_fonts() {
    let l = lay(r"\text{if } x \mathbf{v} \mathrm{d} \mathbb{R}", false);
    let t = texts(&l);
    assert!(t.contains(&"if "));
    assert!(t.contains(&"ℝ"));
    assert!(l.glyphs.iter().any(|g| g.face == Face::Bold));
}

#[test]
fn matrices_and_aligned() {
    let m = lay(r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}", true);
    let (a, b, c) = (
        m.glyphs.iter().find(|g| g.text == "a").unwrap(),
        m.glyphs.iter().find(|g| g.text == "b").unwrap(),
        m.glyphs.iter().find(|g| g.text == "c").unwrap(),
    );
    assert!(b.x > a.x && c.y > a.y && (c.x - a.x).abs() < 0.01);
    assert!(!m.segs.is_empty());
    let al = lay(r"\begin{aligned} x &= 1 \\ yy &= 2 \end{aligned}", true);
    let eq: Vec<_> = al.glyphs.iter().filter(|g| g.text == "=").collect();
    assert_eq!(eq.len(), 2);
    assert!((eq[0].x - eq[1].x).abs() < 0.01);
}

#[test]
fn unknown_commands_fall_back_to_source() {
    let l = lay(r"\foo{x} + \begin{weird} a \end{weird}", false);
    let t = texts(&l);
    assert!(t.contains(&"\\foo"), "{t:?}");
    assert!(t.iter().any(|s| s.contains("\\begin{weird}")), "{t:?}");
}

#[test]
fn spacing_commands_and_garbage_do_not_panic() {
    let wide = lay(r"a\quad b", false).width;
    let narrow = lay("ab", false).width;
    assert!(wide > narrow + 10.0);
    for s in [
        "",
        "{",
        "}",
        "^",
        "_",
        r"\frac",
        r"\frac{a}",
        r"\left(",
        r"\begin{pmatrix}",
        "x^^2",
        r"\sqrt[3]",
        "&",
        r"\\",
        "{{{{{{{{{{",
    ] {
        let l = lay(s, true);
        assert!(l.width.is_finite() && l.height().is_finite(), "{s}");
    }
}

#[test]
fn deep_nesting_is_bounded() {
    let s = "{".repeat(5000) + "x" + &"}".repeat(5000);
    let l = lay(&s, false);
    assert!(l.width.is_finite());
}
