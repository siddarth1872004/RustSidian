//! LaTeX math subset: parse and lay out into positioned glyphs and line segments.
//!
//! Unsupported commands are kept as their source text instead of failing.

mod layout;
mod parse;
mod syms;

pub use layout::{layout as layout_nodes, Approx, Glyph, Layout, Measure, Seg};
pub use parse::{parse, Face, Node};

/// Parse and lay out `src` at font size `size`. `display` selects display style (block math).
pub fn render(src: &str, size: f32, display: bool, m: &mut dyn Measure) -> Layout {
    layout_nodes(&parse(src), size, display, m)
}

#[cfg(test)]
mod tests;
