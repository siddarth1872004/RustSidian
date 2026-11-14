//! Line icons for the interface, drawn on a 16 x 16 grid with display-list commands.
//! An icon is an element with a `data-icon` attribute; it is painted after layout in the
//! element's text color, so hover and active colors from the theme apply.

use rustsidian_ui::{Cmd, Dom, El};

type Line = Vec<(f32, f32)>;

#[derive(Default)]
struct Shape {
    lines: Vec<Line>,
    dots: Vec<[f32; 3]>,
}

impl Shape {
    fn l(mut self, pts: &[(f32, f32)]) -> Shape {
        self.lines.push(pts.to_vec());
        self
    }
    fn rect(self, x0: f32, y0: f32, x1: f32, y1: f32) -> Shape {
        self.l(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)])
    }
    fn dot(mut self, x: f32, y: f32, r: f32) -> Shape {
        self.dots.push([x, y, r]);
        self
    }
    /// An arc from `a0` to `a1` degrees (0 is right, 90 is down).
    fn arc(mut self, cx: f32, cy: f32, r: f32, a0: f32, a1: f32) -> Shape {
        let n = (((a1 - a0).abs() / 18.0).ceil() as usize).max(2);
        self.lines.push(
            (0..=n)
                .map(|i| {
                    let a = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
                    (cx + r * a.cos(), cy + r * a.sin())
                })
                .collect(),
        );
        self
    }
    fn ring(self, cx: f32, cy: f32, r: f32) -> Shape {
        self.arc(cx, cy, r, 0.0, 360.0)
    }
    /// Short strokes pointing outward from a circle, like sun rays or gear teeth.
    fn rays(mut self, cx: f32, cy: f32, r0: f32, r1: f32, count: usize) -> Shape {
        for i in 0..count {
            let a = (i as f32 / count as f32 * 360.0).to_radians();
            self.lines
                .push(vec![(cx + r0 * a.cos(), cy + r0 * a.sin()), (cx + r1 * a.cos(), cy + r1 * a.sin())]);
        }
        self
    }
}

const FOLDER: [(f32, f32); 7] =
    [(1.5, 3.5), (6.2, 3.5), (7.8, 5.5), (14.5, 5.5), (14.5, 12.5), (1.5, 12.5), (1.5, 3.5)];
const PAGE: [(f32, f32); 6] = [(3.5, 1.5), (9.5, 1.5), (12.5, 4.5), (12.5, 14.5), (3.5, 14.5), (3.5, 1.5)];
const PAGE_FOLD: [(f32, f32); 3] = [(9.5, 1.5), (9.5, 4.5), (12.5, 4.5)];

fn shape(name: &str) -> Shape {
    let s = Shape::default();
    match name {
        "folder" => s.l(&FOLDER),
        "folder-plus" => s.l(&FOLDER).l(&[(8.0, 7.3), (8.0, 10.9)]).l(&[(6.2, 9.1), (9.8, 9.1)]),
        "file" => s.l(&PAGE).l(&PAGE_FOLD),
        "note" => s.l(&PAGE).l(&PAGE_FOLD).l(&[(5.8, 8.0), (10.2, 8.0)]).l(&[(5.8, 11.0), (10.2, 11.0)]),
        "note-plus" => s.l(&PAGE).l(&PAGE_FOLD).l(&[(8.0, 7.2), (8.0, 11.8)]).l(&[(5.7, 9.5), (10.3, 9.5)]),
        "image" => s.rect(2.0, 3.0, 14.0, 13.0).dot(5.6, 6.4, 1.1).l(&[
            (2.5, 11.8),
            (6.3, 8.6),
            (8.8, 10.6),
            (10.8, 9.0),
            (13.5, 11.4),
        ]),
        "canvas" => {
            s.rect(1.8, 2.2, 7.2, 6.4).rect(8.8, 9.6, 14.2, 13.8).l(&[(7.2, 4.3), (11.5, 4.3), (11.5, 9.6)])
        }
        "search" => s.ring(7.0, 7.0, 4.4).l(&[(10.2, 10.2), (14.0, 14.0)]),
        "switcher" => s
            .l(&[(2.0, 5.5), (13.0, 5.5)])
            .l(&[(10.0, 2.5), (13.0, 5.5), (10.0, 8.5)])
            .l(&[(14.0, 10.5), (3.0, 10.5)])
            .l(&[(6.0, 7.5), (3.0, 10.5), (6.0, 13.5)]),
        "command" => s.l(&[(2.5, 4.0), (6.5, 8.0), (2.5, 12.0)]).l(&[(8.2, 12.5), (13.5, 12.5)]),
        "graph" => s
            .l(&[(4.0, 4.2), (12.0, 5.2), (8.0, 12.0), (4.0, 4.2)])
            .dot(4.0, 4.2, 1.9)
            .dot(12.0, 5.2, 1.9)
            .dot(8.0, 12.0, 1.9),
        "settings" => s.ring(8.0, 8.0, 2.1).ring(8.0, 8.0, 4.6).rays(8.0, 8.0, 4.6, 6.6, 8),
        "sun" => s.ring(8.0, 8.0, 2.9).rays(8.0, 8.0, 4.9, 6.7, 8),
        "moon" => s.arc(8.0, 8.0, 5.6, -55.0, 215.0).arc(11.6, 4.4, 5.0, 70.0, 200.0),
        "plus" => s.l(&[(8.0, 3.0), (8.0, 13.0)]).l(&[(3.0, 8.0), (13.0, 8.0)]),
        "x" => s.l(&[(4.2, 4.2), (11.8, 11.8)]).l(&[(11.8, 4.2), (4.2, 11.8)]),
        "chevron-right" => s.l(&[(6.0, 3.8), (10.2, 8.0), (6.0, 12.2)]),
        "chevron-down" => s.l(&[(3.8, 6.0), (8.0, 10.2), (12.2, 6.0)]),
        "arrow-left" => s.l(&[(13.0, 8.0), (3.0, 8.0)]).l(&[(7.0, 4.0), (3.0, 8.0), (7.0, 12.0)]),
        "arrow-right" => s.l(&[(3.0, 8.0), (13.0, 8.0)]).l(&[(9.0, 4.0), (13.0, 8.0), (9.0, 12.0)]),
        "arrow-up" => s.l(&[(8.0, 13.0), (8.0, 3.0)]).l(&[(4.0, 7.0), (8.0, 3.0), (12.0, 7.0)]),
        "edit" => s
            .l(&[(2.5, 13.5), (3.2, 10.5), (11.0, 2.7), (13.3, 5.0), (5.5, 12.8), (2.5, 13.5)])
            .l(&[(9.5, 4.2), (11.8, 6.5)]),
        "book" => s.l(&[(8.0, 4.0), (8.0, 13.5)]).l(&[
            (8.0, 4.0),
            (5.5, 2.8),
            (1.5, 2.8),
            (1.5, 12.3),
            (5.5, 12.3),
            (8.0, 13.5),
            (10.5, 12.3),
            (14.5, 12.3),
            (14.5, 2.8),
            (10.5, 2.8),
            (8.0, 4.0),
        ]),
        "code" => s.l(&[(5.5, 4.0), (1.8, 8.0), (5.5, 12.0)]).l(&[(10.5, 4.0), (14.2, 8.0), (10.5, 12.0)]),
        "list" => s
            .l(&[(5.5, 4.0), (14.0, 4.0)])
            .l(&[(5.5, 8.0), (14.0, 8.0)])
            .l(&[(5.5, 12.0), (14.0, 12.0)])
            .dot(2.6, 4.0, 0.9)
            .dot(2.6, 8.0, 0.9)
            .dot(2.6, 12.0, 0.9),
        "link" => s
            .l(&[(6.4, 9.6), (9.6, 6.4)])
            .l(&[(7.2, 5.2), (8.8, 3.6), (10.8, 3.0), (12.6, 4.0), (13.0, 6.0), (12.2, 7.6), (10.8, 8.9)])
            .l(&[(8.8, 10.8), (7.2, 12.4), (5.2, 13.0), (3.4, 12.0), (3.0, 10.0), (3.8, 8.4), (5.2, 7.1)]),
        "calendar" => s
            .rect(2.0, 3.5, 14.0, 14.0)
            .l(&[(2.0, 6.8), (14.0, 6.8)])
            .l(&[(5.0, 2.0), (5.0, 5.0)])
            .l(&[(11.0, 2.0), (11.0, 5.0)]),
        "vault" => s.rect(2.0, 2.5, 14.0, 13.5).ring(8.0, 8.0, 2.6).l(&[(8.0, 5.4), (8.0, 8.0)]),
        "collapse" => s.l(&[(4.5, 2.8), (8.0, 6.3), (11.5, 2.8)]).l(&[(4.5, 13.2), (8.0, 9.7), (11.5, 13.2)]),
        "home" => s.l(&[(2.2, 8.2), (8.0, 2.6), (13.8, 8.2)]).l(&[
            (4.0, 7.0),
            (4.0, 13.5),
            (12.0, 13.5),
            (12.0, 7.0),
        ]),
        "drive" => s.rect(2.0, 5.0, 14.0, 11.0).dot(11.6, 8.0, 0.9),
        "check" => s.l(&[(3.0, 8.5), (6.5, 12.0), (13.0, 4.5)]),
        "dot" => s.dot(8.0, 8.0, 2.6),
        "sidebar-right" => s.rect(2.0, 3.0, 14.0, 13.0).l(&[(10.0, 3.0), (10.0, 13.0)]),
        "help" => s
            .ring(8.0, 8.0, 6.0)
            .l(&[(6.2, 6.4), (6.8, 5.0), (8.2, 4.6), (9.6, 5.4), (9.6, 6.8), (8.0, 8.2), (8.0, 9.4)])
            .dot(8.0, 11.4, 0.8),
        "copy" => {
            s.rect(5.5, 5.5, 14.0, 14.0).l(&[(10.5, 5.5), (10.5, 2.0), (2.0, 2.0), (2.0, 10.5), (5.5, 10.5)])
        }
        "clipboard" => s
            .l(&[(5.5, 3.0), (3.0, 3.0), (3.0, 14.5), (13.0, 14.5), (13.0, 3.0), (10.5, 3.0)])
            .rect(5.5, 1.5, 10.5, 4.5),
        "scissors" => s
            .ring(4.5, 12.0, 2.2)
            .ring(11.5, 12.0, 2.2)
            .l(&[(6.0, 10.4), (12.5, 2.0)])
            .l(&[(10.0, 10.4), (3.5, 2.0)]),
        "trash" => s
            .l(&[(2.0, 4.0), (14.0, 4.0)])
            .l(&[(6.0, 4.0), (6.0, 2.0), (10.0, 2.0), (10.0, 4.0)])
            .l(&[(3.5, 4.0), (4.5, 14.5), (11.5, 14.5), (12.5, 4.0)])
            .l(&[(6.6, 7.0), (6.8, 11.8)])
            .l(&[(9.4, 7.0), (9.2, 11.8)]),
        "bold" => s
            .l(&[(4.5, 2.5), (9.5, 2.5), (11.5, 3.6), (11.5, 6.6), (9.5, 7.8), (4.5, 7.8), (4.5, 2.5)])
            .l(&[(9.5, 7.8), (12.2, 9.0), (12.2, 12.3), (10.0, 13.5), (4.5, 13.5), (4.5, 7.8)]),
        "italic" => {
            s.l(&[(6.5, 2.5), (12.5, 2.5)]).l(&[(3.5, 13.5), (9.5, 13.5)]).l(&[(9.5, 2.5), (6.5, 13.5)])
        }
        "strikethrough" => s
            .l(&[(2.0, 8.0), (14.0, 8.0)])
            .l(&[(12.0, 3.5), (10.0, 2.5), (6.0, 2.5), (4.0, 3.8), (4.0, 6.0), (6.0, 7.2)])
            .l(&[(10.6, 9.4), (12.0, 10.6), (12.0, 12.4), (10.0, 13.5), (6.0, 13.5), (4.0, 12.5)]),
        "highlight" => s.l(&[(3.0, 14.5), (13.0, 14.5)]).l(&[
            (4.0, 12.5),
            (5.0, 9.5),
            (11.0, 3.5),
            (13.0, 5.5),
            (7.0, 11.5),
            (4.0, 12.5),
        ]),
        "table" => s
            .rect(2.0, 2.5, 14.0, 13.5)
            .l(&[(2.0, 6.2), (14.0, 6.2)])
            .l(&[(2.0, 9.8), (14.0, 9.8)])
            .l(&[(8.0, 2.5), (8.0, 13.5)]),
        "select" => s
            .l(&[(2.0, 5.0), (2.0, 2.0), (5.0, 2.0)])
            .l(&[(11.0, 2.0), (14.0, 2.0), (14.0, 5.0)])
            .l(&[(14.0, 11.0), (14.0, 14.0), (11.0, 14.0)])
            .l(&[(5.0, 14.0), (2.0, 14.0), (2.0, 11.0)])
            .l(&[(5.5, 6.0), (10.5, 6.0)])
            .l(&[(5.5, 10.0), (10.5, 10.0)]),
        _ => s.rect(3.0, 3.0, 13.0, 13.0),
    }
}

/// An icon element. Its size comes from the `svg-icon` class (16 px unless a theme says otherwise).
pub fn el(name: &str) -> El {
    El::div().class("svg-icon").attr("data-icon", name)
}

/// After layout: turn every icon element into drawing commands.
pub fn paint(dom: &mut Dom) {
    for node in &mut dom.nodes {
        let Some(name) = node.el.attrs.iter().find(|(k, _)| k == "data-icon").map(|(_, v)| v.as_str()) else {
            continue;
        };
        let r = node.rect;
        if r.w <= 0.0 || r.h <= 0.0 || node.style.display_none {
            continue;
        }
        let k = r.w.min(r.h) / 16.0;
        let (ox, oy) = (r.x + (r.w - 16.0 * k) / 2.0, r.y + (r.h - 16.0 * k) / 2.0);
        let color = node.style.text.color.with_alpha(node.style.opacity);
        let shape = shape(name);
        let segs: Vec<[f32; 4]> = shape
            .lines
            .iter()
            .flat_map(|l| l.windows(2))
            .map(|w| [ox + w[0].0 * k, oy + w[0].1 * k, ox + w[1].0 * k, oy + w[1].1 * k])
            .collect();
        let mut cmds = Vec::new();
        if !segs.is_empty() {
            cmds.push(Cmd::Lines { segs, width: 1.3, color });
        }
        if !shape.dots.is_empty() {
            let items = shape.dots.iter().map(|d| [ox + d[0] * k, oy + d[1] * k, d[2] * k]).collect();
            cmds.push(Cmd::Circles { items, color });
        }
        node.draw = cmds;
    }
}

/// The icon for a file in the explorer, by extension.
pub fn for_file(path: &str) -> &'static str {
    match vault_core::path::extension(path).to_ascii_lowercase().as_str() {
        "md" => "note",
        "canvas" => "canvas",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" => "image",
        _ => "file",
    }
}
