//! CSS values: colors, lengths, `calc()`, transitions.

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Color = Color { r: 0, g: 0, b: 0, a: 0 };
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }
    pub fn with_alpha(self, a: f32) -> Color {
        Color { a: (self.a as f32 * a.clamp(0.0, 1.0)).round() as u8, ..self }
    }
    /// Linear interpolation in sRGB, for transitions.
    pub fn lerp(self, to: Color, t: f32) -> Color {
        let f = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t.clamp(0.0, 1.0)).round() as u8;
        Color { r: f(self.r, to.r), g: f(self.g, to.g), b: f(self.b, to.b), a: f(self.a, to.a) }
    }
}

const NAMED: &[(&str, u32)] = &[
    ("black", 0x000000),
    ("white", 0xffffff),
    ("red", 0xff0000),
    ("green", 0x008000),
    ("blue", 0x0000ff),
    ("yellow", 0xffff00),
    ("orange", 0xffa500),
    ("purple", 0x800080),
    ("pink", 0xffc0cb),
    ("gray", 0x808080),
    ("grey", 0x808080),
    ("silver", 0xc0c0c0),
    ("maroon", 0x800000),
    ("navy", 0x000080),
    ("teal", 0x008080),
    ("cyan", 0x00ffff),
    ("aqua", 0x00ffff),
    ("magenta", 0xff00ff),
    ("fuchsia", 0xff00ff),
    ("lime", 0x00ff00),
    ("olive", 0x808000),
    ("brown", 0xa52a2a),
    ("gold", 0xffd700),
    ("indigo", 0x4b0082),
    ("violet", 0xee82ee),
    ("crimson", 0xdc143c),
    ("coral", 0xff7f50),
    ("salmon", 0xfa8072),
    ("tomato", 0xff6347),
    ("khaki", 0xf0e68c),
    ("lightgray", 0xd3d3d3),
    ("lightgrey", 0xd3d3d3),
    ("darkgray", 0xa9a9a9),
    ("darkgrey", 0xa9a9a9),
    ("dimgray", 0x696969),
    ("whitesmoke", 0xf5f5f5),
    ("lightblue", 0xadd8e6),
    ("darkblue", 0x00008b),
    ("darkred", 0x8b0000),
    ("darkgreen", 0x006400),
    ("skyblue", 0x87ceeb),
    ("steelblue", 0x4682b4),
    ("slategray", 0x708090),
    ("rebeccapurple", 0x663399),
    ("turquoise", 0x40e0d0),
    ("beige", 0xf5f5dc),
];

fn args(inner: &str) -> Vec<&str> {
    inner.split([',', '/', ' ']).map(str::trim).filter(|s| !s.is_empty()).collect()
}

fn channel(s: &str, scale: f32) -> Option<f32> {
    match s.strip_suffix('%') {
        Some(p) => p.trim().parse::<f32>().ok().map(|v| v / 100.0 * scale),
        None => s.parse::<f32>().ok(),
    }
}

fn hsl(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let h = h.rem_euclid(360.0) / 360.0;
    let (s, l) = (s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let f = |mut t: f32| {
        t = t.rem_euclid(1.0);
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    (f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0))
}

pub fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    if let Some(hex) = lower.strip_prefix('#') {
        if !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let v = |i: usize, n: usize| {
            u8::from_str_radix(&hex[i..i + n], 16).ok().map(|x| if n == 1 { x * 17 } else { x })
        };
        return match hex.len() {
            3 => Some(Color { r: v(0, 1)?, g: v(1, 1)?, b: v(2, 1)?, a: 255 }),
            4 => Some(Color { r: v(0, 1)?, g: v(1, 1)?, b: v(2, 1)?, a: v(3, 1)? }),
            6 => Some(Color { r: v(0, 2)?, g: v(2, 2)?, b: v(4, 2)?, a: 255 }),
            8 => Some(Color { r: v(0, 2)?, g: v(2, 2)?, b: v(4, 2)?, a: v(6, 2)? }),
            _ => None,
        };
    }
    if lower == "transparent" {
        return Some(Color::TRANSPARENT);
    }
    let func = |name: &str| {
        lower.strip_prefix(name).and_then(|r| r.strip_prefix('(')).and_then(|r| r.strip_suffix(')'))
    };
    if let Some(inner) = func("rgba").or_else(|| func("rgb")) {
        let a = args(inner);
        if a.len() < 3 {
            return None;
        }
        let c = |i: usize| channel(a[i], 255.0).map(|v| v.round().clamp(0.0, 255.0) as u8);
        let alpha = a.get(3).map_or(Some(1.0), |x| channel(x, 1.0))?;
        return Some(Color {
            r: c(0)?,
            g: c(1)?,
            b: c(2)?,
            a: (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
        });
    }
    if let Some(inner) = func("hsla").or_else(|| func("hsl")) {
        let a = args(inner);
        if a.len() < 3 {
            return None;
        }
        let h: f32 = a[0].trim_end_matches("deg").parse().ok()?;
        let (r, g, b) = hsl(h, channel(a[1], 1.0)?, channel(a[2], 1.0)?);
        let alpha = a.get(3).map_or(Some(1.0), |x| channel(x, 1.0))?;
        return Some(Color { r, g, b, a: (alpha.clamp(0.0, 1.0) * 255.0).round() as u8 });
    }
    NAMED
        .iter()
        .find(|(n, _)| *n == lower)
        .map(|&(_, v)| Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// Context for resolving relative lengths.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LengthCx {
    pub font_size: f32,
    pub root_font_size: f32,
    /// What `100%` means for the property being resolved.
    pub percent_base: f32,
    pub viewport: (f32, f32),
}

impl Default for LengthCx {
    fn default() -> Self {
        LengthCx { font_size: 16.0, root_font_size: 16.0, percent_base: 0.0, viewport: (1280.0, 800.0) }
    }
}

fn unit(v: f32, unit: &str, cx: &LengthCx) -> Option<f32> {
    Some(match unit {
        "px" | "" => v,
        "em" => v * cx.font_size,
        "rem" => v * cx.root_font_size,
        "%" => v / 100.0 * cx.percent_base,
        "pt" => v * 96.0 / 72.0,
        "vw" => v / 100.0 * cx.viewport.0,
        "vh" => v / 100.0 * cx.viewport.1,
        "ch" => v * cx.font_size * 0.5,
        "ex" => v * cx.font_size * 0.5,
        "ms" => v,
        "s" => v * 1000.0,
        _ => return None,
    })
}

struct Calc<'a> {
    b: &'a [u8],
    s: &'a str,
    i: usize,
    cx: &'a LengthCx,
    depth: u32,
}

impl Calc<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn sum(&mut self) -> Option<f32> {
        let mut v = self.product()?;
        loop {
            self.ws();
            match self.b.get(self.i) {
                Some(b'+') => {
                    self.i += 1;
                    v += self.product()?;
                }
                Some(b'-') => {
                    self.i += 1;
                    v -= self.product()?;
                }
                _ => return Some(v),
            }
        }
    }
    fn product(&mut self) -> Option<f32> {
        let mut v = self.atom()?;
        loop {
            self.ws();
            match self.b.get(self.i) {
                Some(b'*') => {
                    self.i += 1;
                    v *= self.atom()?;
                }
                Some(b'/') => {
                    self.i += 1;
                    let d = self.atom()?;
                    if d == 0.0 {
                        return None;
                    }
                    v /= d;
                }
                _ => return Some(v),
            }
        }
    }
    fn atom(&mut self) -> Option<f32> {
        self.ws();
        self.depth += 1;
        if self.depth > 64 {
            return None;
        }
        let rest = &self.s[self.i..];
        let lower_start = rest.get(..6).unwrap_or(rest).to_ascii_lowercase();
        let func = ["calc(", "min(", "max(", "clamp(", "("].into_iter().find(|f| lower_start.starts_with(f));
        let v = if let Some(f) = func {
            self.i += f.len();
            let mut vals = vec![self.sum()?];
            loop {
                self.ws();
                match self.b.get(self.i) {
                    Some(b',') => {
                        self.i += 1;
                        vals.push(self.sum()?);
                    }
                    Some(b')') => {
                        self.i += 1;
                        break;
                    }
                    _ => return None,
                }
            }
            match f {
                "min(" => vals.into_iter().reduce(f32::min)?,
                "max(" => vals.into_iter().reduce(f32::max)?,
                "clamp(" if vals.len() == 3 => vals[1].max(vals[0]).min(vals[2]),
                "clamp(" => return None,
                _ if vals.len() == 1 => vals[0],
                _ => return None,
            }
        } else {
            let start = self.i;
            if matches!(self.b.get(self.i), Some(b'-' | b'+')) {
                self.i += 1;
            }
            while self.i < self.b.len() && (self.b[self.i].is_ascii_digit() || self.b[self.i] == b'.') {
                self.i += 1;
            }
            let num: f32 = self.s[start..self.i].parse().ok()?;
            let ustart = self.i;
            while self.i < self.b.len() && (self.b[self.i].is_ascii_alphabetic() || self.b[self.i] == b'%') {
                self.i += 1;
            }
            unit(num, &self.s[ustart..self.i].to_ascii_lowercase(), self.cx)?
        };
        self.depth -= 1;
        Some(v)
    }
}

/// A length, number or time in px / unitless / ms. Handles `calc()`, `min()`, `max()`, `clamp()`.
pub fn parse_length(s: &str, cx: &LengthCx) -> Option<f32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut c = Calc { b: s.as_bytes(), s, i: 0, cx, depth: 0 };
    let v = c.sum()?;
    c.ws();
    (c.i == s.len() && v.is_finite()).then_some(v)
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Easing {
    Linear,
    Bezier(f32, f32, f32, f32),
}

impl Easing {
    pub fn parse(s: &str) -> Option<Easing> {
        Some(match s.trim() {
            "linear" => Easing::Linear,
            "ease" => Easing::Bezier(0.25, 0.1, 0.25, 1.0),
            "ease-in" => Easing::Bezier(0.42, 0.0, 1.0, 1.0),
            "ease-out" => Easing::Bezier(0.0, 0.0, 0.58, 1.0),
            "ease-in-out" => Easing::Bezier(0.42, 0.0, 0.58, 1.0),
            other => {
                let inner = other.strip_prefix("cubic-bezier(")?.strip_suffix(')')?;
                let v: Vec<f32> = inner.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if v.len() != 4 {
                    return None;
                }
                Easing::Bezier(v[0].clamp(0.0, 1.0), v[1], v[2].clamp(0.0, 1.0), v[3])
            }
        })
    }

    /// Progress (0..=1) at time fraction `t` (0..=1).
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let Easing::Bezier(x1, y1, x2, y2) = self else { return t };
        let curve = |a: f32, b: f32, u: f32| {
            3.0 * a * u * (1.0 - u) * (1.0 - u) + 3.0 * b * u * u * (1.0 - u) + u * u * u
        };
        // Solve x(u) = t by bisection; x is monotonic for x1, x2 in [0, 1].
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if curve(x1, x2, mid) < t {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        curve(y1, y2, (lo + hi) / 2.0)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Transition {
    /// Property name or `all`.
    pub property: Box<str>,
    pub duration_ms: f32,
    pub delay_ms: f32,
    pub easing: Easing,
}

/// Parse the `transition` shorthand.
pub fn parse_transitions(value: &str) -> Vec<Transition> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    let mut parts = Vec::new();
    for (i, c) in value.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&value[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&value[start..]);
    for part in parts {
        let mut t = Transition {
            property: "all".into(),
            duration_ms: 0.0,
            delay_ms: 0.0,
            easing: Easing::Bezier(0.25, 0.1, 0.25, 1.0),
        };
        let mut times = 0;
        let mut tokens: Vec<String> = Vec::new();
        let mut cur = String::new();
        let mut d = 0;
        for c in part.trim().chars() {
            match c {
                '(' => d += 1,
                ')' => d -= 1,
                _ => {}
            }
            if c.is_whitespace() && d == 0 {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
            } else {
                cur.push(c);
            }
        }
        if !cur.is_empty() {
            tokens.push(cur);
        }
        if tokens.is_empty() || tokens[0] == "none" {
            continue;
        }
        for tok in tokens {
            let is_time = (tok.ends_with("ms") || tok.ends_with('s')) && tok.as_bytes()[0].is_ascii_digit()
                || tok.starts_with('.');
            if is_time {
                if let Some(ms) = parse_length(&tok, &LengthCx::default()) {
                    if times == 0 {
                        t.duration_ms = ms;
                    } else {
                        t.delay_ms = ms;
                    }
                    times += 1;
                    continue;
                }
            }
            match Easing::parse(&tok) {
                Some(e) => t.easing = e,
                None => t.property = tok.into(),
            }
        }
        out.push(t);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        let c = |s| parse_color(s).unwrap();
        assert_eq!(c("#fff"), Color::rgb(255, 255, 255));
        assert_eq!(c("#1e1e1e80"), Color { r: 30, g: 30, b: 30, a: 128 });
        assert_eq!(c("#F0a8"), Color { r: 255, g: 0, b: 170, a: 136 });
        assert_eq!(c("rgb(255, 0, 128)"), Color::rgb(255, 0, 128));
        assert_eq!(c("rgba(10,20,30,0.5)"), Color { r: 10, g: 20, b: 30, a: 128 });
        assert_eq!(c("rgb(10 20 30 / 50%)"), Color { r: 10, g: 20, b: 30, a: 128 });
        assert_eq!(c("rgb(100%, 0%, 50%)"), Color::rgb(255, 0, 128));
        assert_eq!(c("hsl(0, 100%, 50%)"), Color::rgb(255, 0, 0));
        assert_eq!(c("hsl(258, 88%, 66%)"), Color::rgb(138, 92, 245));
        assert_eq!(c("hsla(120deg 100% 25% / 0.2)"), Color { r: 0, g: 128, b: 0, a: 51 });
        assert_eq!(c("RebeccaPurple"), Color::rgb(102, 51, 153));
        assert_eq!(c("transparent").a, 0);
        for bad in ["", "#12", "#ggg", "rgb(1,2)", "notacolor", "rgb(a,b,c)"] {
            assert_eq!(parse_color(bad), None, "{bad}");
        }
        assert_eq!(Color::rgb(0, 0, 0).lerp(Color::rgb(100, 200, 255), 0.5), Color::rgb(50, 100, 128));
    }

    #[test]
    fn lengths_and_calc() {
        let cx = LengthCx {
            font_size: 20.0,
            root_font_size: 16.0,
            percent_base: 200.0,
            viewport: (1000.0, 500.0),
        };
        let l = |s| parse_length(s, &cx);
        assert_eq!(l("12px"), Some(12.0));
        assert_eq!(l("1.5em"), Some(30.0));
        assert_eq!(l("2rem"), Some(32.0));
        assert_eq!(l("50%"), Some(100.0));
        assert_eq!(l("10vw"), Some(100.0));
        assert_eq!(l("0"), Some(0.0));
        assert_eq!(l("-4px"), Some(-4.0));
        assert_eq!(l("calc(100% - 2 * 10px)"), Some(180.0));
        assert_eq!(l("calc((1em + 4px) / 2)"), Some(12.0));
        assert_eq!(l("max(10px, 1em)"), Some(20.0));
        assert_eq!(l("clamp(10px, 50%, 60px)"), Some(60.0));
        assert_eq!(l("min(5px, calc(1px + 1px))"), Some(2.0));
        assert_eq!(l("200ms"), Some(200.0));
        assert_eq!(l("0.2s"), Some(200.0));
        for bad in ["auto", "calc(1px +)", "12 px", "calc(1px / 0)", "1furlong", "(((((("] {
            assert_eq!(l(bad), None, "{bad}");
        }
    }

    #[test]
    fn transitions_and_easing() {
        let t = parse_transitions(
            "opacity 0.2s ease-in-out, background-color 150ms cubic-bezier(0.4, 0, 0.2, 1) 50ms, none",
        );
        assert_eq!(t.len(), 2);
        assert_eq!((&*t[0].property, t[0].duration_ms), ("opacity", 200.0));
        assert_eq!((&*t[1].property, t[1].duration_ms, t[1].delay_ms), ("background-color", 150.0, 50.0));
        assert_eq!(t[1].easing, Easing::Bezier(0.4, 0.0, 0.2, 1.0));
        assert_eq!(parse_transitions("200ms")[0].property.as_ref(), "all");
        let e = Easing::parse("ease-in-out").unwrap();
        assert!(
            (e.apply(0.0)).abs() < 1e-3
                && (e.apply(1.0) - 1.0).abs() < 1e-3
                && (e.apply(0.5) - 0.5).abs() < 1e-2
        );
        assert!(Easing::parse("ease-in").unwrap().apply(0.3) < 0.3);
        assert_eq!(Easing::Linear.apply(0.4), 0.4);
    }
}
