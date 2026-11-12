//! Mermaid diagrams without JavaScript: `graph`/`flowchart` with a layered layout and
//! `sequenceDiagram`. Everything else is reported as unsupported so the caller can show the source.

#![allow(clippy::needless_range_loop)]

mod flow;
mod seq;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fill {
    None,
    Node,
    Accent,
    Background,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Item {
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        fill: Fill,
        stroke: bool,
    },
    Diamond {
        cx: f32,
        cy: f32,
        w: f32,
        h: f32,
        fill: Fill,
    },
    Ellipse {
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        fill: Fill,
    },
    Line {
        pts: Vec<(f32, f32)>,
        dashed: bool,
        width: f32,
    },
    /// Filled triangle with its tip at `(x, y)` pointing along `angle` (radians).
    Arrow {
        x: f32,
        y: f32,
        angle: f32,
        open: bool,
    },
    /// Text centered on `(x, y)`.
    Text {
        x: f32,
        y: f32,
        text: String,
        bold: bool,
        size: f32,
    },
}

#[derive(Clone, Default, Debug)]
pub struct Diagram {
    pub width: f32,
    pub height: f32,
    pub items: Vec<Item>,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Error {
    Unsupported(String),
    Syntax(String),
}

pub trait Measure {
    /// Width of one line of text at `size` (bold for node titles).
    fn width(&mut self, text: &str, size: f32, bold: bool) -> f32;
}

/// Font-independent widths, for sizing before real fonts exist.
pub struct Approx;

impl Measure for Approx {
    fn width(&mut self, text: &str, size: f32, bold: bool) -> f32 {
        let n: f32 = text
            .chars()
            .map(|c| match c {
                ' ' | 'i' | 'l' | 'j' | '.' | ',' | ':' | '\'' | '|' | '!' => 0.3,
                'f' | 't' | 'r' | '(' | ')' | '[' | ']' => 0.38,
                'm' | 'w' | 'M' | 'W' => 0.85,
                'A'..='Z' => 0.68,
                c if c.len_utf8() > 1 => 1.0,
                _ => 0.56,
            })
            .sum();
        n * size * if bold { 1.07 } else { 1.0 }
    }
}

pub const TEXT_SIZE: f32 = 14.0;

/// Lay out the diagram described by `src` (the contents of a ```mermaid fence).
pub fn render(src: &str, m: &mut dyn Measure) -> Result<Diagram, Error> {
    let lines: Vec<&str> =
        src.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("%%")).collect();
    let mut start = 0;
    if lines.first().is_some_and(|l| *l == "---") {
        if let Some(end) = lines[1..].iter().position(|l| *l == "---") {
            start = end + 2;
        }
    }
    let Some(first) = lines.get(start) else { return Err(Error::Syntax("empty diagram".into())) };
    let word = first.split(|c: char| c.is_whitespace() || c == ';').next().unwrap_or("");
    match word {
        "graph" | "flowchart" | "flowchart-elk" => flow::render(first, &lines[start + 1..], m),
        "sequenceDiagram" => seq::render(&lines[start + 1..], m),
        other => Err(Error::Unsupported(other.to_string())),
    }
}

#[cfg(test)]
mod tests;
