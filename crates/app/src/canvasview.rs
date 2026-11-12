//! Canvas view: a `.canvas` file drawn in the main pane. Nodes and edges are painted as a display
//! list in window coordinates, the same way the graph view does it.

use crate::shell::{App, Focus};
use canvas_core::{Canvas, Edge, Node, NodeKind, Side};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use theme_engine::Color;
use vault_core::fsio::{self, Stamp};
use vault_core::{path, WriteError};
use walnut_ui::{Cmd, Dom, El, NodeId, Rect, Span, TextStyle};

/// Hidden elements the colors are read from, in order: node fill, border, text, faint text,
/// edge, accent, then the six canvas presets.
pub const COLOR_CLASSES: [&str; 12] = [
    "color-node",
    "color-border",
    "color-text",
    "color-faint",
    "color-edge",
    "color-accent",
    "color-1",
    "color-2",
    "color-3",
    "color-4",
    "color-5",
    "color-6",
];

const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 4.0;
const MIN_SIZE: f64 = 60.0;
const GRAB: f32 = 6.0;
const TITLE_H: f64 = 28.0;
const GROUP_HEAD: f64 = 30.0;
const DOUBLE_CLICK: Duration = Duration::from_millis(450);
const PREVIEW_CHARS: usize = 700;

enum Drag {
    Pan { last: (f32, f32) },
    Move { id: String, last: [f64; 2] },
    Resize { id: String, sides: [bool; 4], start: [f64; 2], orig: [f64; 4] },
}

struct Press {
    drag: Drag,
    at: (f32, f32),
    moved: bool,
    open: Option<String>,
}

struct FileInfo {
    title: String,
    body: Option<String>,
    image: Option<walnut_ui::Image>,
    missing: bool,
}

pub struct CanvasView {
    pub path: String,
    pub canvas: Canvas,
    stamp: Stamp,
    pub cam: [f32; 2],
    pub zoom: f32,
    pub auto_fit: bool,
    pub selected: Option<String>,
    pub editing: Option<String>,
    pub dirty: bool,
    press: Option<Press>,
    last_click: Option<(Instant, String)>,
    files: HashMap<String, FileInfo>,
    seed: u64,
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color { r: m(a.r, b.r), g: m(a.g, b.g), b: m(a.b, b.b), a: 255 }
}

fn fill(rect: Rect, color: Color, radius: f32) -> Cmd {
    Cmd::Fill { rect, color, radius }
}

/// Plain text plus styled spans for the common Markdown a canvas card holds.
fn markdown_lite(src: &str, accent: Color, faint: Color) -> (String, Vec<Span>) {
    let mut out = String::new();
    let mut spans = Vec::new();
    let base = Span { scale: 1.0, ..Span::default() };
    let mut fence = false;
    for line in src.lines() {
        if line.trim_start().starts_with("```") {
            fence = !fence;
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        let start = out.len();
        if fence {
            out.push_str(line);
            spans.push(Span { start, end: out.len(), mono: true, ..base.clone() });
            continue;
        }
        let t = line.trim_start();
        let hashes = t.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
            let scale = [1.6, 1.4, 1.25, 1.1, 1.0, 1.0][hashes - 1];
            let style = Span { bold: true, scale, ..base.clone() };
            inline(&mut out, &mut spans, t[hashes + 1..].trim(), &style, accent);
        } else if let Some(rest) = t.strip_prefix("> ") {
            let style = Span { italic: true, color: Some(faint), ..base.clone() };
            inline(&mut out, &mut spans, rest, &style, accent);
        } else if let Some(rest) = ["- ", "* ", "+ "].iter().find_map(|m| t.strip_prefix(m)) {
            out.push_str(&line[..line.len() - t.len()]);
            out.push_str("• ");
            inline(&mut out, &mut spans, rest, &base, accent);
        } else {
            inline(&mut out, &mut spans, line, &base, accent);
        }
    }
    (out, spans)
}

fn inline(out: &mut String, spans: &mut Vec<Span>, s: &str, base: &Span, accent: Color) {
    const PAIRS: [(&str, &str); 6] = [("**", "**"), ("~~", "~~"), ("`", "`"), ("*", "*"), ("_", "_"), ("[[", "]]")];
    fn push(out: &mut String, spans: &mut Vec<Span>, text: &str, style: Span) {
        let start = out.len();
        out.push_str(text);
        spans.push(Span { start, end: out.len(), ..style });
    }
    let mut rest = s;
    while !rest.is_empty() {
        let hit = PAIRS.iter().find_map(|(open, close)| {
            let body = rest.strip_prefix(open)?;
            let end = body.find(close).filter(|&e| e > 0)?;
            Some((*open, end, open.len() + end + close.len()))
        });
        if let Some((open, end, used)) = hit {
            let body = &rest[open.len()..open.len() + end];
            let style = match open {
                "**" => Span { bold: true, ..base.clone() },
                "~~" => Span { strike: true, ..base.clone() },
                "`" => Span { mono: true, ..base.clone() },
                "[[" => Span { color: Some(accent), ..base.clone() },
                _ => Span { italic: true, ..base.clone() },
            };
            let shown = if open == "[[" { body.rsplit('|').next().unwrap_or(body) } else { body };
            push(out, spans, shown, style);
            rest = &rest[used..];
            continue;
        }
        if let Some((label, url)) = rest
            .strip_prefix('[')
            .and_then(|r| r.split_once("]("))
            .and_then(|(l, r)| Some((l, r.split_once(')')?.0)))
        {
            push(out, spans, label, Span { color: Some(accent), underline: true, ..base.clone() });
            rest = &rest[1 + label.len() + 2 + url.len() + 1..];
            continue;
        }
        let n = rest.chars().next().map_or(1, char::len_utf8);
        let next = rest[n..].find(['*', '_', '`', '~', '[']).map_or(rest.len(), |i| i + n);
        push(out, spans, &rest[..next], base.clone());
        rest = &rest[next..];
    }
}

fn side_dir(s: Side) -> (f32, f32) {
    match s {
        Side::Top => (0.0, -1.0),
        Side::Bottom => (0.0, 1.0),
        Side::Left => (-1.0, 0.0),
        Side::Right => (1.0, 0.0),
    }
}

/// The sides an edge without explicit sides attaches to: those facing each other.
fn facing(a: &Node, b: &Node) -> (Side, Side) {
    let dx = (b.x + b.width / 2.0) - (a.x + a.width / 2.0);
    let dy = (b.y + b.height / 2.0) - (a.y + a.height / 2.0);
    match (dx.abs() > dy.abs(), dx > 0.0, dy > 0.0) {
        (true, true, _) => (Side::Right, Side::Left),
        (true, false, _) => (Side::Left, Side::Right),
        (false, _, true) => (Side::Bottom, Side::Top),
        (false, _, false) => (Side::Top, Side::Bottom),
    }
}

impl CanvasView {
    pub fn open(path: &str, text: &str, stamp: Stamp) -> Result<CanvasView, canvas_core::Error> {
        Ok(CanvasView {
            path: path.to_string(),
            canvas: Canvas::parse(text)?,
            stamp,
            cam: [0.0; 2],
            zoom: 1.0,
            auto_fit: true,
            selected: None,
            editing: None,
            dirty: false,
            press: None,
            last_click: None,
            files: HashMap::new(),
            seed: fsio::now_ms() | 1,
        })
    }

    pub fn title(&self) -> String {
        path::stem(&self.path).to_string()
    }

    fn fit(&mut self, rect: &Rect) {
        if !self.auto_fit {
            return;
        }
        match self.canvas.bounds() {
            Some((x, y, w, h)) => {
                let zx = (rect.w - 80.0) / w.max(1.0) as f32;
                let zy = (rect.h - 80.0) / h.max(1.0) as f32;
                self.zoom = zx.min(zy).clamp(MIN_ZOOM, 1.0);
                self.cam = [(x + w / 2.0) as f32, (y + h / 2.0) as f32];
            }
            None => {
                self.zoom = 1.0;
                self.cam = [0.0; 2];
            }
        }
    }

    pub fn to_screen(&self, rect: &Rect, x: f64, y: f64) -> (f32, f32) {
        (
            rect.x + rect.w / 2.0 + (x as f32 - self.cam[0]) * self.zoom,
            rect.y + rect.h / 2.0 + (y as f32 - self.cam[1]) * self.zoom,
        )
    }

    pub fn to_world(&self, rect: &Rect, x: f32, y: f32) -> [f64; 2] {
        [
            (self.cam[0] + (x - rect.x - rect.w / 2.0) / self.zoom) as f64,
            (self.cam[1] + (y - rect.y - rect.h / 2.0) / self.zoom) as f64,
        ]
    }

    pub fn node_rect(&self, rect: &Rect, n: &Node) -> Rect {
        let (x, y) = self.to_screen(rect, n.x, n.y);
        Rect::new(x, y, n.width as f32 * self.zoom, n.height as f32 * self.zoom)
    }

    /// The node under a world point; a group only answers on its header strip so that the rest
    /// of it pans the canvas.
    fn node_at(&self, w: [f64; 2]) -> Option<&Node> {
        self.canvas.hit(w[0], w[1]).filter(|n| !matches!(n.kind, NodeKind::Group { .. }) || w[1] <= n.y + GROUP_HEAD)
    }

    fn commit_edit(&mut self) {
        let Some(text) = self.editing.take() else { return };
        let Some(id) = self.selected.clone() else { return };
        if let Some(Node { kind: NodeKind::Text { text: old }, .. }) = self.canvas.node_mut(&id) {
            if *old != text {
                *old = text;
                self.dirty = true;
            }
        }
    }

    fn start_edit(&mut self) {
        let Some(id) = self.selected.clone() else { return };
        if let Some(Node { kind: NodeKind::Text { text }, .. }) = self.canvas.node(&id) {
            self.editing = Some(text.clone());
        }
    }

    pub fn press(&mut self, rect: &Rect, x: f32, y: f32) {
        let w = self.to_world(rect, x, y);
        if self.editing.is_some() {
            let inside = self.selected.as_deref().and_then(|id| self.canvas.node(id)).is_some_and(|n| n.contains(w[0], w[1]));
            if inside {
                return;
            }
            self.commit_edit();
        }
        if let Some(sel) = self.selected.as_deref().and_then(|id| self.canvas.node(id)) {
            let r = self.node_rect(rect, sel);
            let near = |a: f32, b: f32| (a - b).abs() <= GRAB;
            let within = |v: f32, lo: f32, hi: f32| v >= lo - GRAB && v <= hi + GRAB;
            if within(x, r.x, r.x + r.w) && within(y, r.y, r.y + r.h) {
                let sides = [near(x, r.x), near(x, r.x + r.w), near(y, r.y), near(y, r.y + r.h)];
                if sides.iter().any(|&s| s) {
                    let drag = Drag::Resize { id: sel.id.clone(), sides, start: w, orig: [sel.x, sel.y, sel.width, sel.height] };
                    self.press = Some(Press { drag, at: (x, y), moved: false, open: None });
                    return;
                }
            }
        }
        let hit = self.node_at(w).map(|n| (n.id.clone(), n.kind.clone(), n.y));
        let Some((id, kind, top)) = hit else {
            self.selected = None;
            self.last_click = None;
            self.press = Some(Press { drag: Drag::Pan { last: (x, y) }, at: (x, y), moved: false, open: None });
            return;
        };
        let double = self.last_click.as_ref().is_some_and(|(t, last)| *last == id && t.elapsed() < DOUBLE_CLICK);
        self.last_click = Some((Instant::now(), id.clone()));
        self.selected = Some(id.clone());
        if double && matches!(kind, NodeKind::Text { .. }) {
            self.start_edit();
            self.last_click = None;
            return;
        }
        let open = match kind {
            NodeKind::File { file, .. } if w[1] - top <= TITLE_H => Some(file),
            _ => None,
        };
        self.press = Some(Press { drag: Drag::Move { id, last: w }, at: (x, y), moved: false, open });
    }

    pub fn drag(&mut self, rect: &Rect, x: f32, y: f32) {
        let w = self.to_world(rect, x, y);
        let Some(press) = &mut self.press else { return };
        if !press.moved && (x - press.at.0).abs() + (y - press.at.1).abs() < 3.0 {
            return;
        }
        press.moved = true;
        match &mut press.drag {
            Drag::Pan { last } => {
                self.cam[0] -= (x - last.0) / self.zoom;
                self.cam[1] -= (y - last.1) / self.zoom;
                *last = (x, y);
                self.auto_fit = false;
            }
            Drag::Move { id, last } => {
                let (dx, dy) = (w[0] - last[0], w[1] - last[1]);
                *last = w;
                self.canvas.move_node(id, dx, dy);
                self.dirty = true;
            }
            Drag::Resize { id, sides, start, orig } => {
                let (dx, dy) = (w[0] - start[0], w[1] - start[1]);
                let [ox, oy, ow, oh] = *orig;
                if let Some(n) = self.canvas.node_mut(id) {
                    if sides[1] {
                        n.width = (ow + dx).max(MIN_SIZE);
                    }
                    if sides[0] {
                        n.width = (ow - dx).max(MIN_SIZE);
                        n.x = ox + ow - n.width;
                    }
                    if sides[3] {
                        n.height = (oh + dy).max(MIN_SIZE);
                    }
                    if sides[2] {
                        n.height = (oh - dy).max(MIN_SIZE);
                        n.y = oy + oh - n.height;
                    }
                }
                self.dirty = true;
            }
        }
    }

    /// The file whose title was clicked, if the press ended without a drag.
    pub fn release(&mut self) -> Option<String> {
        let p = self.press.take()?;
        if p.moved {
            None
        } else {
            p.open
        }
    }

    pub fn wheel(&mut self, rect: &Rect, x: f32, y: f32, lines: f32) {
        let before = self.to_world(rect, x, y);
        self.zoom = (self.zoom * 1.1f32.powf(-lines)).clamp(MIN_ZOOM, MAX_ZOOM);
        self.cam[0] = before[0] as f32 - (x - rect.x - rect.w / 2.0) / self.zoom;
        self.cam[1] = before[1] as f32 - (y - rect.y - rect.h / 2.0) / self.zoom;
        self.auto_fit = false;
    }

    fn color_of(spec: &Option<String>, c: &[Color; 12]) -> Option<Color> {
        let s = spec.as_deref()?;
        match s.parse::<usize>() {
            Ok(i @ 1..=6) => Some(c[5 + i]),
            _ => theme_engine::parse_color(s),
        }
    }

    pub fn paint(&mut self, rect: Rect, c: &[Color; 12], text: &TextStyle) -> Vec<Cmd> {
        self.fit(&rect);
        let z = self.zoom;
        let mut cmds = vec![Cmd::PushClip(rect)];
        let view = Rect::new(rect.x - 20.0, rect.y - 20.0, rect.w + 40.0, rect.h + 40.0);
        let overlaps = |r: &Rect| r.x < view.x + view.w && r.x + r.w > view.x && r.y < view.y + view.h && r.y + r.h > view.y;
        let accent = c[5];
        let selected = self.selected.as_deref();
        for n in self.canvas.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Group { .. })) {
            let r = self.node_rect(&rect, n);
            let label_h = 16.0 * z;
            if !overlaps(&Rect::new(r.x, r.y - label_h, r.w, r.h + label_h)) {
                continue;
            }
            let col = Self::color_of(&n.color, c).unwrap_or(c[1]);
            let sel = selected == Some(n.id.as_str());
            let bw = if sel { 3.0 } else { 2.0 };
            cmds.push(fill(r, if sel { accent } else { col.with_alpha(0.55) }, 10.0 * z));
            let inner = Rect::new(r.x + bw, r.y + bw, r.w - 2.0 * bw, r.h - 2.0 * bw);
            cmds.push(fill(inner, mix(c[0], col, 0.1), 8.0 * z));
            if let NodeKind::Group { label: Some(l) } = &n.kind {
                if z > 0.12 {
                    cmds.push(Cmd::Text {
                        x: r.x + 4.0,
                        y: r.y - label_h - 2.0,
                        width: Some(r.w),
                        text: l.clone(),
                        spans: Vec::new(),
                        style: TextStyle { size: 13.0 * z, line_height: 16.0 * z, color: c[3], wrap: false, ..text.clone() },
                    });
                }
            }
        }
        for e in &self.canvas.edges {
            self.paint_edge(&rect, e, c, text, &mut cmds);
        }
        for n in self.canvas.nodes.iter().filter(|n| !matches!(n.kind, NodeKind::Group { .. })) {
            let r = self.node_rect(&rect, n);
            if !overlaps(&r) {
                continue;
            }
            let col = Self::color_of(&n.color, c);
            let sel = selected == Some(n.id.as_str());
            let bw = if sel { 3.0 } else { 1.5 };
            let border = if sel { accent } else { col.unwrap_or(c[1]) };
            let bg = col.map_or(c[0], |k| mix(c[0], k, 0.1));
            cmds.push(fill(r, border, 6.0 * z));
            let inner = Rect::new(r.x + bw, r.y + bw, r.w - 2.0 * bw, r.h - 2.0 * bw);
            cmds.push(fill(inner, bg, (6.0 * z - bw).max(0.0)));
            if z < 0.2 {
                continue;
            }
            let pad = 10.0 * z;
            let body = Rect::new(inner.x + pad, inner.y + pad * 0.8, (inner.w - 2.0 * pad).max(1.0), (inner.h - 1.6 * pad).max(1.0));
            let base = TextStyle { size: 14.0 * z, line_height: 20.0 * z, color: c[2], wrap: true, ..text.clone() };
            cmds.push(Cmd::PushClip(inner));
            match &n.kind {
                NodeKind::Text { text: src } => {
                    let editing = if sel { self.editing.as_ref() } else { None };
                    let (t, spans) = match editing {
                        Some(buf) => (format!("{buf}|"), Vec::new()),
                        None => markdown_lite(src, accent, c[3]),
                    };
                    cmds.push(Cmd::Text { x: body.x, y: body.y, width: Some(body.w), text: t, spans, style: base });
                }
                NodeKind::Link { url } => {
                    let span = Span { start: 0, end: url.len(), color: Some(accent), underline: true, scale: 1.0, ..Span::default() };
                    cmds.push(Cmd::Text { x: body.x, y: body.y, width: Some(body.w), text: url.clone(), spans: vec![span], style: base });
                }
                NodeKind::File { file, subpath } => {
                    let info = self.files.get(file.as_str());
                    let title = match (info, subpath) {
                        (Some(i), Some(s)) => format!("{} {s}", i.title),
                        (Some(i), None) => i.title.clone(),
                        _ => file.clone(),
                    };
                    let th = 24.0 * z;
                    let tcolor = if info.is_some_and(|i| i.missing) { c[3] } else { accent };
                    let tstyle = TextStyle { bold: true, color: tcolor, wrap: false, ..base.clone() };
                    cmds.push(Cmd::Text { x: body.x, y: body.y, width: Some(body.w), text: title, spans: Vec::new(), style: tstyle });
                    if let Some(i) = info {
                        let below = Rect::new(body.x, body.y + th, body.w, (body.h - th).max(1.0));
                        if let Some(img) = &i.image {
                            let (iw, ih) = (img.0.width() as f32, img.0.height() as f32);
                            let s = (below.w / iw).min(below.h / ih);
                            let (w, h) = (iw * s, ih * s);
                            let at = Rect::new(below.x + (below.w - w) / 2.0, below.y, w, h);
                            cmds.push(Cmd::Image { rect: at, image: img.clone() });
                        } else if let Some(b) = &i.body {
                            let (t, spans) = markdown_lite(b, accent, c[3]);
                            cmds.push(Cmd::PushClip(below));
                            cmds.push(Cmd::Text { x: below.x, y: below.y, width: Some(below.w), text: t, spans, style: base });
                            cmds.push(Cmd::PopClip);
                        }
                    }
                }
                _ => {}
            }
            cmds.push(Cmd::PopClip);
        }
        cmds.push(Cmd::PopClip);
        cmds
    }

    fn paint_edge(&self, rect: &Rect, e: &Edge, c: &[Color; 12], text: &TextStyle, cmds: &mut Vec<Cmd>) {
        let (Some(a), Some(b)) = (self.canvas.node(&e.from_node), self.canvas.node(&e.to_node)) else { return };
        let auto = facing(a, b);
        let (sa, sb) = (e.from_side.unwrap_or(auto.0), e.to_side.unwrap_or(auto.1));
        let z = self.zoom;
        let (p0, p3) = (a.anchor(sa), b.anchor(sb));
        let (p0, p3) = (self.to_screen(rect, p0.0, p0.1), self.to_screen(rect, p3.0, p3.1));
        let dist = ((p3.0 - p0.0).powi(2) + (p3.1 - p0.1).powi(2)).sqrt();
        let d = (dist * 0.5).clamp(40.0 * z, 200.0 * z);
        let (da, db) = (side_dir(sa), side_dir(sb));
        let p1 = (p0.0 + da.0 * d, p0.1 + da.1 * d);
        let p2 = (p3.0 + db.0 * d, p3.1 + db.1 * d);
        let (minx, maxx) = (p0.0.min(p3.0).min(p1.0).min(p2.0), p0.0.max(p3.0).max(p1.0).max(p2.0));
        let (miny, maxy) = (p0.1.min(p3.1).min(p1.1).min(p2.1), p0.1.max(p3.1).max(p1.1).max(p2.1));
        if maxx < rect.x || minx > rect.x + rect.w || maxy < rect.y || miny > rect.y + rect.h {
            return;
        }
        let col = Self::color_of(&e.color, c).unwrap_or(c[4]);
        let at = |t: f32| {
            let u = 1.0 - t;
            let (k0, k1, k2, k3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            (k0 * p0.0 + k1 * p1.0 + k2 * p2.0 + k3 * p3.0, k0 * p0.1 + k1 * p1.1 + k2 * p2.1 + k3 * p3.1)
        };
        let steps = ((dist / 12.0).ceil() as usize).clamp(4, 40);
        let mut segs = Vec::with_capacity(steps);
        let mut prev = p0;
        for i in 1..=steps {
            let q = at(i as f32 / steps as f32);
            segs.push([prev.0, prev.1, q.0, q.1]);
            prev = q;
        }
        let arrow = |tip: (f32, f32), dir: (f32, f32), segs: &mut Vec<[f32; 4]>| {
            let s = (11.0 * z).clamp(6.0, 22.0);
            let back = (tip.0 - dir.0 * s, tip.1 - dir.1 * s);
            for k in -4i32..=4 {
                let off = k as f32 / 4.0 * s * 0.45;
                segs.push([tip.0, tip.1, back.0 - dir.1 * off, back.1 + dir.0 * off]);
            }
        };
        let mut heads = Vec::new();
        if e.to_arrow {
            arrow(p3, (-db.0, -db.1), &mut heads);
        }
        if e.from_arrow {
            arrow(p0, (-da.0, -da.1), &mut heads);
        }
        cmds.push(Cmd::Lines { segs, width: (2.0 * z).clamp(1.0, 4.0), color: col });
        if !heads.is_empty() {
            cmds.push(Cmd::Lines { segs: heads, width: 1.5, color: col });
        }
        if let Some(label) = e.label.as_deref().filter(|l| !l.is_empty() && z > 0.2) {
            let size = 13.0 * z;
            let mid = at(0.5);
            let w = label.chars().count() as f32 * size * 0.56 + 10.0 * z;
            let h = size * 1.5;
            let r = Rect::new(mid.0 - w / 2.0, mid.1 - h / 2.0, w, h);
            cmds.push(fill(r, c[0], 4.0 * z));
            cmds.push(Cmd::Text {
                x: r.x + 5.0 * z,
                y: r.y + h * 0.15,
                width: None,
                text: label.to_string(),
                spans: Vec::new(),
                style: TextStyle { size, line_height: size * 1.2, color: c[2], wrap: false, ..text.clone() },
            });
        }
    }
}

/// The canvas pane and the hidden elements its colors are read from.
pub fn pane(c: &CanvasView, dom: &mut Dom, parent: NodeId, info: &mut crate::view::ViewInfo) {
    let header = dom.add(parent, El::div().class("view-header graph-controls canvas-controls"));
    dom.add(header, El::div().class("graph-count").text(format!("{}  ·  {} nodes", c.path, c.canvas.nodes.len())));
    for (label, cmd) in [("Add text", "canvas:add-text"), ("Delete", "canvas:delete-selected"), ("Fit", "canvas:reset-view")] {
        dom.add(header, El::div().class("graph-control").text(label).action(cmd));
    }
    let pane = dom.add(parent, El::div().class("view-content canvas-wrapper"));
    let colors = dom.add(pane, El::div().class("canvas-view-colors"));
    info.canvas_colors = COLOR_CLASSES.iter().map(|k| dom.add(colors, El::div().class("canvas-view").class(k))).collect();
    info.canvas = Some(pane);
}

/// After layout: draw the canvas into its pane.
pub fn add_canvas(app: &mut App, dom: &mut Dom, info: &crate::view::ViewInfo) {
    let Some(pane) = info.canvas else { return };
    app.canvas_resolve_files();
    let Some(v) = &mut app.canvas_view else { return };
    let mut colors = [Color::rgb(128, 128, 128); 12];
    for (slot, &n) in colors.iter_mut().zip(&info.canvas_colors) {
        let s = &dom.nodes[n].style;
        *slot = s.text.color.with_alpha(s.opacity);
    }
    let text = dom.nodes[pane].style.text.clone();
    dom.nodes[pane].draw = v.paint(dom.nodes[pane].rect, &colors, &text);
}

fn strip_front_matter(s: &str) -> &str {
    match s.strip_prefix("---\n").and_then(|r| r.find("\n---").map(|i| &r[i + 4..])) {
        Some(rest) => rest.trim_start_matches(['\n', '\r']),
        None => s,
    }
}

impl App {
    pub fn canvas_active(&self) -> bool {
        self.show_canvas && !self.show_graph && self.canvas_view.is_some()
    }

    pub(crate) fn open_canvas(&mut self, rel: &str) {
        if let Some(v) = &mut self.canvas_view {
            v.commit_edit();
        }
        self.canvas_save();
        if self.canvas_view.as_ref().is_none_or(|c| !c.path.eq_ignore_ascii_case(rel)) {
            let opened = self
                .store
                .read(rel)
                .map_err(|e| e.to_string())
                .and_then(|(text, stamp)| CanvasView::open(rel, &text, stamp).map_err(|e| e.to_string()));
            match opened {
                Ok(v) => self.canvas_view = Some(v),
                Err(e) => {
                    self.notice = Some(format!("Cannot open {rel}: {e}"));
                    return;
                }
            }
        }
        self.show_canvas = true;
        self.show_graph = false;
        self.focus = Focus::Editor;
    }

    pub(crate) fn canvas_save(&mut self) {
        let Some(v) = self.canvas_view.as_mut().filter(|v| v.dirty) else { return };
        match self.store.write(&v.path, &v.canvas.to_json(), Some(v.stamp)) {
            Ok(stamp) => {
                v.stamp = stamp;
                v.dirty = false;
            }
            Err(WriteError::Conflict { .. }) => {
                self.notice = Some(format!("{} was changed outside Rustwalnut. Not saved.", v.path))
            }
            Err(WriteError::Io(e)) => self.notice = Some(format!("Save failed: {e}")),
        }
    }

    pub(crate) fn canvas_files_changed(&mut self, paths: &[String]) {
        let Some(v) = self.canvas_view.as_mut() else { return };
        v.files.clear();
        if v.dirty || !paths.iter().any(|p| p.eq_ignore_ascii_case(&v.path)) {
            return;
        }
        if let Ok((text, stamp)) = self.store.read(&v.path) {
            if text != v.canvas.to_json() {
                if let Ok(canvas) = Canvas::parse(&text) {
                    v.canvas = canvas;
                    v.editing = None;
                }
            }
            v.stamp = stamp;
        }
    }

    pub(crate) fn canvas_resolve_files(&mut self) {
        let Some(v) = self.canvas_view.as_mut() else { return };
        for n in &v.canvas.nodes {
            let NodeKind::File { file, .. } = &n.kind else { continue };
            if v.files.contains_key(file) {
                continue;
            }
            let rel = path::normalize(file);
            let found = self.store.index.get(&rel).is_some();
            let is_md = path::kind_of(&rel) == vault_core::FileKind::Markdown;
            let image = if found && !is_md { self.media.borrow_mut().image(&self.store, &rel) } else { None };
            let body = if found && is_md { self.media.borrow_mut().note(&self.store, &rel) } else { None }
                .map(|t| strip_front_matter(&t).chars().take(PREVIEW_CHARS).collect::<String>());
            let title = if is_md { path::stem(&rel).to_string() } else { path::file_name(&rel).to_string() };
            v.files.insert(file.clone(), FileInfo { title, body, image, missing: !found });
        }
    }

    pub(crate) fn run_canvas(&mut self, command: &str) -> bool {
        match command {
            "canvas:focus" => {
                self.show_canvas = self.canvas_view.is_some();
                self.show_graph = false;
            }
            "canvas:reset-view" => {
                if let Some(v) = &mut self.canvas_view {
                    v.auto_fit = true;
                }
            }
            "canvas:add-text" => {
                let Some(v) = &mut self.canvas_view else { return true };
                v.commit_edit();
                let (w, h) = (250.0, 140.0);
                let (x, y) = (v.cam[0] as f64 - w / 2.0, v.cam[1] as f64 - h / 2.0);
                v.seed = v.seed.wrapping_mul(6364136223846793005).wrapping_add(fsio::now_ms());
                let id = v.canvas.add_node(NodeKind::Text { text: String::new() }, x, y, w, h, v.seed);
                v.selected = Some(id);
                v.editing = Some(String::new());
                v.auto_fit = false;
                v.dirty = true;
                self.show_canvas = true;
                self.show_graph = false;
                self.focus = Focus::Editor;
            }
            "canvas:delete-selected" => {
                let Some(v) = &mut self.canvas_view else { return true };
                v.editing = None;
                if let Some(id) = v.selected.take() {
                    v.canvas.remove_node(&id);
                    v.dirty = true;
                }
                self.canvas_save();
            }
            "canvas:edit-selected" => {
                if let Some(v) = &mut self.canvas_view {
                    v.start_edit();
                }
            }
            _ => return false,
        }
        true
    }

    pub(crate) fn canvas_key(&mut self, key: &crate::Key) -> bool {
        let Some(v) = self.canvas_view.as_mut() else { return false };
        if let Some(buf) = &mut v.editing {
            match key.key.as_str() {
                "Escape" => {
                    v.commit_edit();
                    self.canvas_save();
                }
                "Backspace" => {
                    buf.pop();
                }
                "Enter" => buf.push('\n'),
                _ => return false,
            }
            return true;
        }
        match key.key.as_str() {
            "Delete" | "Backspace" => self.run("canvas:delete-selected"),
            "Enter" => self.run("canvas:edit-selected"),
            "Escape" => v.selected = None,
            _ => return false,
        }
        true
    }

    pub(crate) fn canvas_text(&mut self, text: &str) {
        if let Some(buf) = self.canvas_view.as_mut().and_then(|v| v.editing.as_mut()) {
            buf.push_str(text);
        }
    }

    fn canvas_rect(&self, frame: &crate::Frame) -> Option<Rect> {
        if !self.canvas_active() {
            return None;
        }
        frame.info.canvas.map(|n| frame.dom.nodes[n].rect)
    }

    pub fn canvas_click(&mut self, frame: &crate::Frame, x: f32, y: f32) -> bool {
        let Some(rect) = self.canvas_rect(frame) else { return false };
        if let Some(v) = self.canvas_view.as_mut().filter(|_| rect.contains(x, y)) {
            v.press(&rect, x, y);
        }
        true
    }

    pub fn canvas_drag(&mut self, frame: &crate::Frame, x: f32, y: f32) -> bool {
        let Some(rect) = self.canvas_rect(frame) else { return false };
        if let Some(v) = &mut self.canvas_view {
            v.drag(&rect, x, y);
        }
        true
    }

    pub fn canvas_wheel(&mut self, frame: &crate::Frame, x: f32, y: f32, lines: f32) -> bool {
        let Some(rect) = self.canvas_rect(frame).filter(|r| r.contains(x, y)) else { return false };
        if let Some(v) = &mut self.canvas_view {
            v.wheel(&rect, x, y, lines / 3.0);
        }
        true
    }

    pub fn canvas_release(&mut self) {
        let Some(v) = &mut self.canvas_view else { return };
        let open = v.release();
        self.canvas_save();
        if let Some(file) = open {
            self.open_note(&file);
        }
    }
}
