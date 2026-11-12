//! Text shaping, measuring and glyph rasterization (cosmic-text), with a layout cache.

use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style as FontStyle, SwashCache, Weight,
};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use theme_engine::Color;

/// A styled run inside a text block. Offsets are bytes into the block's text.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub color: Option<Color>,
    pub background: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub mono: bool,
    pub strike: bool,
    pub underline: bool,
    /// Font size multiplier (headings).
    pub scale: f32,
}

#[derive(Clone, PartialEq, Debug)]
pub struct TextStyle {
    pub size: f32,
    pub line_height: f32,
    pub color: Color,
    pub bold: bool,
    pub italic: bool,
    pub mono: bool,
    pub wrap: bool,
}

/// A rectangle produced by text decoration or caret queries, relative to the text origin.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TextRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub struct Fonts {
    system: FontSystem,
    swash: SwashCache,
    cache: HashMap<u64, Buffer>,
    sans: String,
    mono: String,
}

const SANS_CANDIDATES: &[&str] = &[
    "Inter",
    "Noto Sans",
    "Segoe UI",
    "DejaVu Sans",
    "Liberation Sans",
    "Ubuntu",
    "Cantarell",
    "Arial",
    "Helvetica Neue",
];
const MONO_CANDIDATES: &[&str] = &[
    "JetBrains Mono",
    "Cascadia Mono",
    "Consolas",
    "DejaVu Sans Mono",
    "Noto Sans Mono",
    "Liberation Mono",
    "Ubuntu Mono",
    "Menlo",
    "Monaco",
];

fn key(text: &str, spans: &[Span], style: &TextStyle, width: Option<f32>) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    for s in spans {
        (s.start, s.end, s.bold, s.italic, s.mono, s.scale.to_bits()).hash(&mut h);
        s.color.map(|c| (c.r, c.g, c.b, c.a)).hash(&mut h);
    }
    (style.size.to_bits(), style.line_height.to_bits(), style.bold, style.italic, style.mono, style.wrap)
        .hash(&mut h);
    (style.color.r, style.color.g, style.color.b, style.color.a).hash(&mut h);
    width.map(|w| (w * 4.0) as i64).hash(&mut h);
    h.finish()
}

impl Default for Fonts {
    fn default() -> Self {
        Fonts::new()
    }
}

impl Fonts {
    /// Loads system fonts. This scan is the slow part of start-up (tens of ms); the app calls
    /// it on a worker thread while the index loads.
    pub fn new() -> Fonts {
        let system = FontSystem::new();
        let has = |name: &str| system.db().faces().any(|f| f.families.iter().any(|(n, _)| n == name));
        let sans = SANS_CANDIDATES.iter().find(|n| has(n)).map_or_else(String::new, |s| s.to_string());
        let mono = MONO_CANDIDATES.iter().find(|n| has(n)).map_or_else(String::new, |s| s.to_string());
        Fonts { system, swash: SwashCache::new(), cache: HashMap::new(), sans, mono }
    }

    pub fn font_count(&self) -> usize {
        self.system.db().len()
    }

    fn attrs<'a>(
        sans: &'a str,
        mono: &'a str,
        color: Color,
        bold: bool,
        italic: bool,
        is_mono: bool,
    ) -> Attrs<'a> {
        let family = match (is_mono, sans.is_empty(), mono.is_empty()) {
            (true, _, false) => Family::Name(mono),
            (true, _, true) => Family::Monospace,
            (false, false, _) => Family::Name(sans),
            (false, true, _) => Family::SansSerif,
        };
        Attrs::new()
            .family(family)
            .weight(if bold { Weight::BOLD } else { Weight::NORMAL })
            .style(if italic { FontStyle::Italic } else { FontStyle::Normal })
            .color(cosmic_text::Color::rgba(color.r, color.g, color.b, color.a))
    }

    fn buffer(&mut self, text: &str, spans: &[Span], style: &TextStyle, width: Option<f32>) -> u64 {
        let k = key(text, spans, style, width);
        if self.cache.contains_key(&k) {
            return k;
        }
        if self.cache.len() > 4000 {
            self.cache.clear();
        }
        let mut buf = Buffer::new(&mut self.system, Metrics::new(style.size, style.line_height));
        buf.set_size(if style.wrap { width } else { None }, None);
        let base = Self::attrs(&self.sans, &self.mono, style.color, style.bold, style.italic, style.mono);
        if spans.is_empty() {
            buf.set_text(text, &base, Shaping::Advanced, None);
        } else {
            // Fill the gaps between spans with the base style.
            let mut runs: Vec<(&str, Attrs)> = Vec::with_capacity(spans.len() * 2 + 1);
            let mut pos = 0;
            for s in spans {
                let (a, b) = (s.start.min(text.len()), s.end.min(text.len()));
                if a < pos || b <= a || !text.is_char_boundary(a) || !text.is_char_boundary(b) {
                    continue;
                }
                if a > pos {
                    runs.push((&text[pos..a], base.clone()));
                }
                let mut at = Self::attrs(
                    &self.sans,
                    &self.mono,
                    s.color.unwrap_or(style.color),
                    style.bold || s.bold,
                    style.italic || s.italic,
                    style.mono || s.mono,
                );
                if s.scale > 0.0 && (s.scale - 1.0).abs() > 0.01 {
                    at = at.metrics(Metrics::new(style.size * s.scale, style.line_height * s.scale));
                }
                runs.push((&text[a..b], at));
                pos = b;
            }
            if pos < text.len() {
                runs.push((&text[pos..], base.clone()));
            }
            buf.set_rich_text(runs, &base, Shaping::Advanced, None);
        }
        buf.shape_until_scroll(&mut self.system, false);
        self.cache.insert(k, buf);
        k
    }

    /// Size of the laid-out text.
    pub fn measure(
        &mut self,
        text: &str,
        spans: &[Span],
        style: &TextStyle,
        width: Option<f32>,
    ) -> (f32, f32) {
        if text.is_empty() {
            return (0.0, style.line_height);
        }
        let k = self.buffer(text, spans, style, width);
        let buf = &self.cache[&k];
        let (mut w, mut h) = (0.0f32, 0.0f32);
        for run in buf.layout_runs() {
            w = w.max(run.line_w);
            h = h.max(run.line_top + run.line_height);
        }
        (w.ceil(), h.max(style.line_height).ceil())
    }

    /// Rectangles covering the byte range `start..end` (selection, highlights, strike-through).
    /// An empty range yields a zero-width caret rectangle.
    pub fn range_rects(
        &mut self,
        text: &str,
        spans: &[Span],
        style: &TextStyle,
        width: Option<f32>,
        start: usize,
        end: usize,
    ) -> Vec<TextRect> {
        if text.is_empty() {
            return vec![TextRect { x: 0.0, y: 0.0, w: 0.0, h: style.line_height }];
        }
        let k = self.buffer(text, spans, style, width);
        let buf = &self.cache[&k];
        // Byte offset of each buffer line within the whole text.
        let mut line_starts = vec![0usize];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        let mut out = Vec::new();
        let mut caret: Option<TextRect> = None;
        for run in buf.layout_runs() {
            let base = line_starts.get(run.line_i).copied().unwrap_or(0);
            let (mut x0, mut x1) = (f32::MAX, f32::MIN);
            for g in run.glyphs {
                let (gs, ge) = (base + g.start, base + g.end);
                if start == end {
                    if gs == start {
                        caret.get_or_insert(TextRect { x: g.x, y: run.line_top, w: 0.0, h: run.line_height });
                    } else if ge == start {
                        caret = Some(TextRect { x: g.x + g.w, y: run.line_top, w: 0.0, h: run.line_height });
                    }
                } else if gs < end && ge > start {
                    x0 = x0.min(g.x);
                    x1 = x1.max(g.x + g.w);
                }
            }
            if start == end && caret.is_none() && run.glyphs.is_empty() && base == start {
                caret = Some(TextRect { x: 0.0, y: run.line_top, w: 0.0, h: run.line_height });
            }
            if x1 > x0 {
                out.push(TextRect { x: x0, y: run.line_top, w: x1 - x0, h: run.line_height });
            }
        }
        if start == end {
            out.push(caret.unwrap_or(TextRect { x: 0.0, y: 0.0, w: 0.0, h: style.line_height }));
        }
        out
    }

    /// Byte offset in `text` nearest to the point (x, y), relative to the text origin.
    pub fn hit(
        &mut self,
        text: &str,
        spans: &[Span],
        style: &TextStyle,
        width: Option<f32>,
        x: f32,
        y: f32,
    ) -> usize {
        if text.is_empty() {
            return 0;
        }
        let k = self.buffer(text, spans, style, width);
        let buf = &self.cache[&k];
        let mut line_starts = vec![0usize];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        let mut best = text.len();
        for run in buf.layout_runs() {
            let base = line_starts.get(run.line_i).copied().unwrap_or(0);
            if y < run.line_top {
                break;
            }
            best = base + run.text.len();
            if y < run.line_top + run.line_height {
                best = run.glyphs.last().map_or(base, |g| base + g.end);
                for g in run.glyphs {
                    if x < g.x + g.w / 2.0 {
                        best = base + g.start;
                        break;
                    }
                }
                break;
            }
        }
        best.min(text.len())
    }

    /// Draw glyph coverage. The callback receives pixel rectangles relative to the text origin.
    pub fn draw(
        &mut self,
        text: &str,
        spans: &[Span],
        style: &TextStyle,
        width: Option<f32>,
        mut f: impl FnMut(i32, i32, u32, u32, Color),
    ) {
        if text.is_empty() {
            return;
        }
        let k = self.buffer(text, spans, style, width);
        let Some(buf) = self.cache.get_mut(&k) else { return };
        let c = cosmic_text::Color::rgba(style.color.r, style.color.g, style.color.b, style.color.a);
        buf.draw(&mut self.system, &mut self.swash, c, |x, y, w, h, col| {
            f(x, y, w, h, Color { r: col.r(), g: col.g(), b: col.b(), a: col.a() });
        });
    }
}
