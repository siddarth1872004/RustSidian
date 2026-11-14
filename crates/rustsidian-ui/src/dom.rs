//! Retained element tree with CSS styling, a flexbox-subset layout and painting.
//!
//! Layout supports `display: flex|block|none`, `flex-direction`, `flex-grow`, `flex-shrink`,
//! `flex-basis`, `width/height` (px, %, auto) with min/max, padding, margin, border, `gap`,
//! `align-items`, `justify-content`, `position: absolute` with insets, and `overflow`.
//! Not supported: grid, wrapping flex lines, floats, inline layout of mixed elements.

use crate::scene::{Cmd, Rect, Scene};
use crate::text::{Fonts, Span, TextStyle};
use theme_engine::{Color, Computed, Element, Env, LengthCx, Theme};

pub type NodeId = usize;

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Dim {
    #[default]
    Auto,
    Px(f32),
    Pct(f32),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Align {
    #[default]
    Stretch,
    Start,
    Center,
    End,
    SpaceBetween,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Style {
    pub display_none: bool,
    pub row: bool,
    pub grow: f32,
    pub shrink: f32,
    pub width: Dim,
    pub height: Dim,
    pub min_w: f32,
    pub max_w: f32,
    pub min_h: f32,
    pub max_h: f32,
    /// top, right, bottom, left
    pub padding: [f32; 4],
    pub margin: [f32; 4],
    pub border: [f32; 4],
    pub border_color: [Color; 4],
    pub background: Color,
    pub radius: f32,
    pub gap: f32,
    pub align: Align,
    pub justify: Align,
    pub clip: bool,
    pub scroll: bool,
    pub absolute: bool,
    pub inset: [Option<f32>; 4],
    pub text: TextStyle,
    pub center_text: bool,
    pub opacity: f32,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            display_none: false,
            row: false,
            grow: 0.0,
            shrink: 1.0,
            width: Dim::Auto,
            height: Dim::Auto,
            min_w: 0.0,
            max_w: f32::MAX,
            min_h: 0.0,
            max_h: f32::MAX,
            padding: [0.0; 4],
            margin: [0.0; 4],
            border: [0.0; 4],
            border_color: [Color::TRANSPARENT; 4],
            background: Color::TRANSPARENT,
            radius: 0.0,
            gap: 0.0,
            align: Align::Stretch,
            justify: Align::Start,
            clip: false,
            scroll: false,
            absolute: false,
            inset: [None; 4],
            text: TextStyle {
                size: 14.0,
                line_height: 20.0,
                color: Color::rgb(220, 220, 220),
                bold: false,
                italic: false,
                mono: false,
                wrap: true,
            },
            center_text: false,
            opacity: 1.0,
        }
    }
}

fn dim(c: &Computed, name: &str, cx: &LengthCx) -> Dim {
    match c.get(name).map(str::trim) {
        None | Some("auto") => Dim::Auto,
        Some(v) => match v.strip_suffix('%').and_then(|p| p.trim().parse::<f32>().ok()) {
            Some(p) => Dim::Pct(p),
            None => theme_engine::parse_length(v, cx).map_or(Dim::Auto, Dim::Px),
        },
    }
}

fn align(v: Option<&str>, default: Align) -> Align {
    match v.map(str::trim) {
        Some("center") => Align::Center,
        Some("flex-start" | "start") => Align::Start,
        Some("flex-end" | "end") => Align::End,
        Some("space-between") => Align::SpaceBetween,
        Some("stretch") => Align::Stretch,
        _ => default,
    }
}

impl Style {
    /// Resolve typed style values from a computed CSS style.
    pub fn from_computed(c: &Computed, parent_font: f32, root_font: f32) -> Style {
        let d = Style::default();
        let base = LengthCx { font_size: parent_font, root_font_size: root_font, ..LengthCx::default() };
        let size = c.length("font-size", &base).unwrap_or(parent_font);
        let cx = LengthCx { font_size: size, ..base };
        let len = |name: &str| c.length(name, &cx);
        let sides = |prefix: &str, suffix: &str| {
            ["top", "right", "bottom", "left"]
                .map(|s| len(&format!("{prefix}-{s}{suffix}")).unwrap_or(0.0).max(0.0))
        };
        let line_height = match c.get("line-height").map(str::trim) {
            Some(v) if v.parse::<f32>().is_ok() => v.parse::<f32>().unwrap_or(1.5) * size,
            Some(v) => theme_engine::parse_length(v, &cx).unwrap_or(size * 1.5),
            None => size * 1.5,
        };
        let family = c.get("font-family").unwrap_or("").to_ascii_lowercase();
        let overflow = |axis: &str| c.get(axis).map(str::trim).unwrap_or("visible").to_string();
        let (ox, oy) = (overflow("overflow-x"), overflow("overflow-y"));
        let display = c.get("display").map(str::trim).unwrap_or("block");
        Style {
            display_none: display == "none",
            row: display.contains("flex")
                && !c.get("flex-direction").is_some_and(|v| v.trim().starts_with("column")),
            grow: c
                .number("flex-grow")
                .or_else(|| c.get("flex").and_then(|f| f.split_whitespace().next()?.parse().ok()))
                .unwrap_or(0.0),
            shrink: c.number("flex-shrink").unwrap_or(1.0),
            width: dim(c, "width", &cx),
            height: dim(c, "height", &cx),
            min_w: len("min-width").unwrap_or(0.0),
            max_w: len("max-width").unwrap_or(f32::MAX),
            min_h: len("min-height").unwrap_or(0.0),
            max_h: len("max-height").unwrap_or(f32::MAX),
            padding: sides("padding", ""),
            margin: sides("margin", ""),
            border: sides("border", "-width"),
            border_color: ["top", "right", "bottom", "left"]
                .map(|s| c.color(&format!("border-{s}-color")).unwrap_or(Color::TRANSPARENT)),
            background: c.color("background-color").unwrap_or(Color::TRANSPARENT),
            radius: len("border-radius").unwrap_or(0.0),
            gap: len(
                if display.contains("flex")
                    && !c.get("flex-direction").is_some_and(|v| v.trim().starts_with("column"))
                {
                    "column-gap"
                } else {
                    "row-gap"
                },
            )
            .unwrap_or(0.0),
            align: align(c.get("align-items"), Align::Stretch),
            justify: align(c.get("justify-content"), Align::Start),
            clip: ox != "visible" || oy != "visible",
            scroll: matches!(oy.as_str(), "auto" | "scroll"),
            absolute: matches!(c.get("position").map(str::trim), Some("absolute" | "fixed")),
            inset: ["top", "right", "bottom", "left"].map(len),
            text: TextStyle {
                size,
                line_height,
                color: c.color("color").unwrap_or(d.text.color),
                bold: c.font_weight() >= 600,
                italic: c.get("font-style").is_some_and(|v| v.trim() == "italic"),
                mono: family.contains("mono") || family.contains("code") || family.contains("courier"),
                wrap: !matches!(c.get("white-space").map(str::trim), Some("nowrap" | "pre")),
            },
            center_text: c.get("text-align").is_some_and(|v| v.trim() == "center"),
            opacity: c.number("opacity").unwrap_or(1.0).clamp(0.0, 1.0),
        }
    }
}

/// Element description used to build the tree.
#[derive(Clone, Debug, Default)]
pub struct El {
    pub tag: &'static str,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<(String, String)>,
    pub text: Option<String>,
    pub spans: Vec<Span>,
    pub inline_style: Option<String>,
    pub action: Option<String>,
    /// Painted over the content box, stretched to fit.
    pub image: Option<crate::scene::Image>,
}

impl El {
    pub fn new(tag: &'static str) -> El {
        El { tag, ..El::default() }
    }
    pub fn div() -> El {
        El::new("div")
    }
    pub fn class(mut self, classes: &str) -> El {
        self.classes.extend(classes.split_whitespace().map(str::to_string));
        self
    }
    pub fn class_if(self, cond: bool, classes: &str) -> El {
        if cond {
            self.class(classes)
        } else {
            self
        }
    }
    pub fn id(mut self, id: &str) -> El {
        self.id = Some(id.to_string());
        self
    }
    pub fn attr(mut self, k: &str, v: &str) -> El {
        self.attrs.push((k.to_string(), v.to_string()));
        self
    }
    pub fn text(mut self, t: impl Into<String>) -> El {
        self.text = Some(t.into());
        self
    }
    pub fn spans(mut self, spans: Vec<Span>) -> El {
        self.spans = spans;
        self
    }
    pub fn image(mut self, image: crate::scene::Image) -> El {
        self.image = Some(image);
        self
    }
    pub fn style(mut self, css: &str) -> El {
        self.inline_style = Some(css.to_string());
        self
    }
    /// Command (with optional argument) dispatched when the element is clicked.
    pub fn action(mut self, a: impl Into<String>) -> El {
        self.action = Some(a.into());
        self
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub el: El,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub style: Style,
    pub computed: Computed,
    /// Border box in window coordinates (after ancestors' scrolling).
    pub rect: Rect,
    pub scroll_y: f32,
    /// Height of the content, for scroll limits and scrollbars.
    pub content_h: f32,
    pub hover: bool,
    pub focus: bool,
    pub active: bool,
    /// Extra rectangles painted under the text, relative to the content box (selections, carets
    /// are painted above via `overlays_top`).
    pub overlays: Vec<(Rect, Color)>,
    pub overlays_top: Vec<(Rect, Color)>,
    /// Extra commands in window coordinates, painted above the node's own content and below
    /// its children. Filled in after layout (custom-drawn views such as the graph).
    pub draw: Vec<Cmd>,
}

#[derive(Clone, Debug)]
pub struct Dom {
    pub nodes: Vec<Node>,
}

#[derive(Clone, Copy)]
struct ElRef<'a>(&'a Dom, NodeId);

impl Element for ElRef<'_> {
    fn tag(&self) -> &str {
        self.0.nodes[self.1].el.tag
    }
    fn id(&self) -> Option<&str> {
        self.0.nodes[self.1].el.id.as_deref()
    }
    fn has_class(&self, class: &str) -> bool {
        self.0.nodes[self.1].el.classes.iter().any(|c| c == class)
    }
    fn attr(&self, name: &str) -> Option<&str> {
        self.0.nodes[self.1].el.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    fn parent(&self) -> Option<Self> {
        self.0.nodes[self.1].parent.map(|p| ElRef(self.0, p))
    }
    fn prev_sibling(&self) -> Option<Self> {
        let p = self.0.nodes[self.1].parent?;
        let sibs = &self.0.nodes[p].children;
        let i = sibs.iter().position(|&c| c == self.1)?;
        (i > 0).then(|| ElRef(self.0, sibs[i - 1]))
    }
    fn is_last_child(&self) -> bool {
        self.0.nodes[self.1].parent.is_some_and(|p| self.0.nodes[p].children.last() == Some(&self.1))
    }
    fn hover(&self) -> bool {
        self.0.nodes[self.1].hover
    }
    fn focus(&self) -> bool {
        self.0.nodes[self.1].focus
    }
    fn active(&self) -> bool {
        self.0.nodes[self.1].active
    }
}

impl Default for Dom {
    fn default() -> Self {
        Dom::new()
    }
}

impl Dom {
    /// A tree with a `body` root (node 0).
    pub fn new() -> Dom {
        let mut d = Dom { nodes: Vec::new() };
        d.push(None, El::new("body"));
        d
    }

    fn push(&mut self, parent: Option<NodeId>, el: El) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node {
            el,
            parent,
            children: Vec::new(),
            style: Style::default(),
            computed: Computed::default(),
            rect: Rect::default(),
            scroll_y: 0.0,
            content_h: 0.0,
            hover: false,
            focus: false,
            active: false,
            overlays: Vec::new(),
            overlays_top: Vec::new(),
            draw: Vec::new(),
        });
        if let Some(p) = parent {
            self.nodes[p].children.push(id);
        }
        id
    }

    pub fn add(&mut self, parent: NodeId, el: El) -> NodeId {
        self.push(Some(parent), el)
    }

    /// Find the first node with this id attribute.
    pub fn by_id(&self, id: &str) -> Option<NodeId> {
        self.nodes.iter().position(|n| n.el.id.as_deref() == Some(id))
    }

    /// Compute styles for every node (parents before children).
    pub fn style(&mut self, theme: &Theme, env: Env) {
        let root_font = 16.0;
        // The cascade is the costly part, and many nodes repeat one (rows, icons, labels).
        // Nodes whose parents are styled alike and that match the same rules with the same
        // inline style get the same result, so it is computed once per such group.
        type Key = (usize, Vec<(u16, u32, u32)>, Option<String>);
        let mut groups: std::collections::HashMap<Key, NodeId> = std::collections::HashMap::new();
        let mut group_of = vec![0; self.nodes.len()];
        for id in 0..self.nodes.len() {
            let parent = self.nodes[id].parent;
            let classes: Vec<&str> = self.nodes[id].el.classes.iter().map(String::as_str).collect();
            let rules = theme.matched_rules(&ElRef(self, id), &classes, env);
            let key =
                (parent.map_or(usize::MAX, |p| group_of[p]), rules, self.nodes[id].el.inline_style.clone());
            if let Some(&same) = groups.get(&key) {
                group_of[id] = same;
                let (computed, style) = (self.nodes[same].computed.clone(), self.nodes[same].style.clone());
                let n = &mut self.nodes[id];
                n.computed = computed;
                n.style = style;
                continue;
            }
            let computed =
                theme.compute_matched(&key.1, parent.map(|p| &self.nodes[p].computed), key.2.as_deref());
            let parent_font = parent.map_or(root_font, |p| self.nodes[p].style.text.size);
            let style = Style::from_computed(&computed, parent_font, root_font);
            group_of[id] = id;
            groups.insert(key, id);
            let n = &mut self.nodes[id];
            n.computed = computed;
            n.style = style;
        }
    }

    fn h_extra(s: &Style) -> f32 {
        s.padding[1] + s.padding[3] + s.border[1] + s.border[3]
    }
    fn v_extra(s: &Style) -> f32 {
        s.padding[0] + s.padding[2] + s.border[0] + s.border[2]
    }

    fn flow_children(&self, id: NodeId) -> Vec<NodeId> {
        self.nodes[id]
            .children
            .iter()
            .copied()
            .filter(|&c| !self.nodes[c].style.display_none && !self.nodes[c].style.absolute)
            .collect()
    }

    /// Preferred border-box size given the widest it may be.
    fn intrinsic(&self, fonts: &mut Fonts, id: NodeId, max_w: f32) -> (f32, f32) {
        let n = &self.nodes[id];
        let s = &n.style;
        let (hx, vx) = (Self::h_extra(s), Self::v_extra(s));
        let fixed_w = match s.width {
            Dim::Px(w) => Some(w),
            Dim::Pct(p) if max_w.is_finite() => Some(max_w * p / 100.0),
            _ => None,
        };
        let avail = (fixed_w.unwrap_or(max_w).min(s.max_w) - hx).max(0.0);
        let (mut w, mut h) = (0.0f32, 0.0f32);
        if let Some(text) = &n.el.text {
            let (tw, th) = fonts.measure(text, &n.el.spans, &s.text, s.text.wrap.then_some(avail));
            (w, h) = (tw, th);
        } else {
            let kids = self.flow_children(id);
            let gaps = s.gap * kids.len().saturating_sub(1) as f32;
            for &c in &kids {
                let m = &self.nodes[c].style.margin;
                let (cw, ch) =
                    self.intrinsic(fonts, c, if s.row { f32::INFINITY } else { avail - m[1] - m[3] });
                let (cw, ch) = (cw + m[1] + m[3], ch + m[0] + m[2]);
                if s.row {
                    w += cw;
                    h = h.max(ch);
                } else {
                    w = w.max(cw);
                    h += ch;
                }
            }
            if s.row {
                w += gaps;
            } else {
                h += gaps;
            }
        }
        let w = fixed_w.unwrap_or(w + hx).clamp(s.min_w, s.max_w);
        let h = match s.height {
            Dim::Px(v) => v,
            _ => h + vx,
        }
        .clamp(s.min_h, s.max_h);
        (w, h)
    }

    /// Lay out the tree for a viewport of `w` x `h` logical pixels.
    pub fn layout(&mut self, fonts: &mut Fonts, w: f32, h: f32) {
        self.place(fonts, 0, Rect::new(0.0, 0.0, w, h));
    }

    /// Assign `rect` (border box) to `id` and lay out its children inside it.
    fn place(&mut self, fonts: &mut Fonts, id: NodeId, rect: Rect) {
        self.nodes[id].rect = rect;
        let s = self.nodes[id].style.clone();
        let inner = Rect::new(
            rect.x + s.border[3] + s.padding[3],
            rect.y + s.border[0] + s.padding[0],
            (rect.w - Self::h_extra(&s)).max(0.0),
            (rect.h - Self::v_extra(&s)).max(0.0),
        );
        let scroll = self.nodes[id].scroll_y;
        let kids = self.flow_children(id);
        let gaps = s.gap * kids.len().saturating_sub(1) as f32;
        // Base main-axis sizes.
        let main_avail = if s.row { inner.w } else { inner.h };
        let mut sizes: Vec<(f32, f32)> = Vec::with_capacity(kids.len());
        let mut used = gaps;
        let mut grow_total = 0.0;
        for &c in &kids {
            let cs = &self.nodes[c].style;
            let m = cs.margin;
            let cross_avail = if s.row { f32::INFINITY } else { inner.w - m[1] - m[3] };
            let (mut cw, mut ch) = self.intrinsic(fonts, c, cross_avail);
            if let Dim::Pct(p) = cs.height {
                ch = inner.h * p / 100.0;
            }
            if let Dim::Pct(p) = cs.width {
                cw = inner.w * p / 100.0;
            }
            if !s.row && (s.align == Align::Stretch || cs.width != Dim::Auto) && cs.width == Dim::Auto {
                cw = (inner.w - m[1] - m[3]).clamp(cs.min_w, cs.max_w);
            }
            used += if s.row { cw + m[1] + m[3] } else { ch + m[0] + m[2] };
            grow_total += cs.grow;
            sizes.push((cw, ch));
        }
        // Distribute free space (grow) or remove overflow (shrink).
        let definite_main = s.row || self.nodes[id].style.height != Dim::Auto || rect.h > 0.0;
        let free = main_avail - used;
        if definite_main && free > 0.0 && grow_total > 0.0 {
            for (i, &c) in kids.iter().enumerate() {
                let g = self.nodes[c].style.grow;
                if g > 0.0 {
                    let add = free * g / grow_total;
                    if s.row {
                        sizes[i].0 += add;
                    } else {
                        sizes[i].1 += add;
                    }
                }
            }
        } else if definite_main && free < 0.0 && !s.scroll {
            // Only children that opted in (grow, clipping or wrapping text) give up space.
            let shrinkable: Vec<usize> = kids
                .iter()
                .enumerate()
                .filter(|(_, &c)| {
                    let cs = &self.nodes[c].style;
                    let fixed = matches!(if s.row { cs.width } else { cs.height }, Dim::Px(_));
                    cs.shrink > 0.0
                        && !fixed
                        && (cs.grow > 0.0
                            || cs.clip
                            || (s.row && self.nodes[c].el.text.is_some() && cs.text.wrap))
                })
                .map(|(i, _)| i)
                .collect();
            let total: f32 = shrinkable.iter().map(|&i| if s.row { sizes[i].0 } else { sizes[i].1 }).sum();
            if total > 0.0 {
                for &i in &shrinkable {
                    let cur = if s.row { sizes[i].0 } else { sizes[i].1 };
                    let new = (cur + free * cur / total).max(0.0);
                    if s.row {
                        sizes[i].0 = new;
                    } else {
                        sizes[i].1 = new;
                    }
                }
            }
        }
        let used: f32 = gaps
            + kids
                .iter()
                .zip(&sizes)
                .map(|(&c, sz)| {
                    let m = self.nodes[c].style.margin;
                    if s.row {
                        sz.0 + m[1] + m[3]
                    } else {
                        sz.1 + m[0] + m[2]
                    }
                })
                .sum::<f32>();
        let leftover = (main_avail - used).max(0.0);
        let (mut cursor, between) = match s.justify {
            Align::Center => (leftover / 2.0, 0.0),
            Align::End => (leftover, 0.0),
            Align::SpaceBetween if kids.len() > 1 => (0.0, leftover / (kids.len() - 1) as f32),
            _ => (0.0, 0.0),
        };
        for (i, &c) in kids.iter().enumerate() {
            let cs = self.nodes[c].style.clone();
            let m = cs.margin;
            let (mut cw, mut ch) = sizes[i];
            let r = if s.row {
                // Text that was squeezed needs its height recomputed at the new width.
                if self.nodes[c].el.text.is_some() || cw < sizes[i].0 {
                    ch = ch.max(self.intrinsic(fonts, c, cw).1);
                }
                let cross = inner.h - m[0] - m[2];
                let y = match s.align {
                    Align::Stretch if cs.height == Dim::Auto && rect.h > 0.0 => {
                        ch = cross.clamp(cs.min_h, cs.max_h);
                        0.0
                    }
                    Align::Center => (cross - ch) / 2.0,
                    Align::End => cross - ch,
                    _ => 0.0,
                };
                let r = Rect::new(inner.x + cursor + m[3], inner.y + m[0] + y - scroll, cw, ch);
                cursor += cw + m[1] + m[3] + s.gap + between;
                r
            } else {
                let cross = inner.w - m[1] - m[3];
                let x = match s.align {
                    Align::Center => (cross - cw) / 2.0,
                    Align::End => cross - cw,
                    _ => 0.0,
                };
                if s.align == Align::Stretch && cs.width == Dim::Auto {
                    cw = cross.clamp(cs.min_w, cs.max_w);
                    // Wrapped text may need more height at the final width.
                    if self.nodes[c].el.text.is_some() && cs.height == Dim::Auto && cs.grow == 0.0 {
                        ch = self.intrinsic(fonts, c, cw).1;
                    }
                }
                let r = Rect::new(inner.x + m[3] + x, inner.y + cursor + m[0] - scroll, cw, ch);
                cursor += ch + m[0] + m[2] + s.gap + between;
                r
            };
            self.place(fonts, c, r);
        }
        self.nodes[id].content_h =
            if s.row { inner.h } else { cursor - if kids.is_empty() { 0.0 } else { s.gap + between } };
        // Absolutely positioned children: relative to this node's padding box.
        let abs: Vec<NodeId> = self.nodes[id]
            .children
            .iter()
            .copied()
            .filter(|&c| self.nodes[c].style.absolute && !self.nodes[c].style.display_none)
            .collect();
        let pad = Rect::new(
            rect.x + s.border[3],
            rect.y + s.border[0],
            rect.w - s.border[1] - s.border[3],
            rect.h - s.border[0] - s.border[2],
        );
        for c in abs {
            let cs = self.nodes[c].style.clone();
            let [top, right, bottom, left] = cs.inset;
            let avail_w = pad.w - left.unwrap_or(0.0) - right.unwrap_or(0.0);
            let (iw, ih) = self.intrinsic(fonts, c, avail_w);
            let w = match (cs.width, left, right) {
                (Dim::Px(w), _, _) => w,
                (Dim::Pct(p), _, _) => pad.w * p / 100.0,
                (Dim::Auto, Some(_), Some(_)) => avail_w,
                _ => iw.min(avail_w.max(0.0)),
            };
            let h = match (cs.height, top, bottom) {
                (Dim::Px(h), _, _) => h,
                (Dim::Pct(p), _, _) => pad.h * p / 100.0,
                (Dim::Auto, Some(t), Some(b)) => pad.h - t - b,
                _ => ih,
            }
            .min(cs.max_h);
            let x = match (left, right) {
                (Some(l), _) => pad.x + l,
                (None, Some(r)) => pad.x + pad.w - r - w,
                _ => pad.x + (pad.w - w) / 2.0,
            };
            let y = match (top, bottom) {
                (Some(t), _) => pad.y + t,
                (None, Some(b)) => pad.y + pad.h - b - h,
                _ => pad.y,
            };
            self.place(fonts, c, Rect::new(x, y, w, h));
        }
    }

    fn content_box(&self, id: NodeId) -> Rect {
        let n = &self.nodes[id];
        let s = &n.style;
        Rect::new(
            n.rect.x + s.border[3] + s.padding[3],
            n.rect.y + s.border[0] + s.padding[0],
            n.rect.w - Self::h_extra(s),
            n.rect.h - Self::v_extra(s),
        )
    }

    pub fn paint(&self) -> Scene {
        let mut scene = Scene::default();
        self.paint_node(0, &mut scene, 1.0);
        scene
    }

    fn paint_node(&self, id: NodeId, scene: &mut Scene, opacity: f32) {
        let n = &self.nodes[id];
        let s = &n.style;
        if s.display_none || n.rect.w <= 0.0 || n.rect.h <= 0.0 {
            return;
        }
        let opacity = opacity * s.opacity;
        let fade = |c: Color| if opacity < 0.999 { c.with_alpha(opacity) } else { c };
        let r = n.rect;
        if s.background.a > 0 {
            scene.cmds.push(Cmd::Fill { rect: r, color: fade(s.background), radius: s.radius });
        }
        let edges = [
            (Rect::new(r.x, r.y, r.w, s.border[0]), 0),
            (Rect::new(r.x + r.w - s.border[1], r.y, s.border[1], r.h), 1),
            (Rect::new(r.x, r.y + r.h - s.border[2], r.w, s.border[2]), 2),
            (Rect::new(r.x, r.y, s.border[3], r.h), 3),
        ];
        for (rect, i) in edges {
            if s.border[i] > 0.0 && s.border_color[i].a > 0 {
                scene.cmds.push(Cmd::Fill { rect, color: fade(s.border_color[i]), radius: 0.0 });
            }
        }
        if s.clip {
            scene.cmds.push(Cmd::PushClip(r));
        }
        let cb = self.content_box(id);
        // Centering needs the measured width; the text was measured during layout and is cached,
        // so this is approximate for wrapped text and exact for single lines.
        let text_dx = match &n.el.text {
            Some(text) if s.center_text => {
                ((cb.w - text.chars().count() as f32 * s.text.size * 0.52) / 2.0).max(0.0)
            }
            _ => 0.0,
        };
        for (rect, color) in &n.overlays {
            scene.cmds.push(Cmd::Fill {
                rect: Rect::new(cb.x + text_dx + rect.x, cb.y + rect.y, rect.w, rect.h),
                color: fade(*color),
                radius: 2.0,
            });
        }
        if let Some(image) = &n.el.image {
            scene.cmds.push(Cmd::Image { rect: cb, image: image.clone() });
        }
        if let Some(text) = &n.el.text {
            let mut style = s.text.clone();
            style.color = fade(style.color);
            scene.cmds.push(Cmd::Text {
                x: cb.x + text_dx,
                y: cb.y,
                width: s.text.wrap.then_some(cb.w),
                text: text.clone(),
                spans: n.el.spans.clone(),
                style,
            });
        }
        for (rect, color) in &n.overlays_top {
            scene.cmds.push(Cmd::Fill {
                rect: Rect::new(cb.x + text_dx + rect.x, cb.y + rect.y, rect.w, rect.h),
                color: fade(*color),
                radius: 0.0,
            });
        }
        scene.cmds.extend(n.draw.iter().cloned());
        for &c in &n.children {
            // Skip children scrolled out of a clipping parent (cheap virtualisation at paint).
            let cr = self.nodes[c].rect;
            if s.clip && (cr.y > r.y + r.h || cr.y + cr.h < r.y) && !self.nodes[c].style.absolute {
                continue;
            }
            self.paint_node(c, scene, opacity);
        }
        if s.scroll && n.content_h > cb.h + 1.0 {
            // Scrollbar thumb.
            let track = r.h;
            let thumb = (track * cb.h / n.content_h).max(24.0);
            let max_scroll = n.content_h - cb.h;
            let y = r.y + (track - thumb) * (n.scroll_y / max_scroll).clamp(0.0, 1.0);
            scene.cmds.push(Cmd::Fill {
                rect: Rect::new(r.x + r.w - 7.0, y, 5.0, thumb),
                color: Color { r: 128, g: 128, b: 128, a: 110 },
                radius: 2.5,
            });
        }
        if s.clip {
            scene.cmds.push(Cmd::PopClip);
        }
    }

    /// Topmost node at a point.
    pub fn hit(&self, x: f32, y: f32) -> Option<NodeId> {
        self.hit_node(0, x, y)
    }

    fn hit_node(&self, id: NodeId, x: f32, y: f32) -> Option<NodeId> {
        let n = &self.nodes[id];
        if n.style.display_none {
            return None;
        }
        let inside = n.rect.contains(x, y);
        if n.style.clip && !inside {
            return None;
        }
        for &c in n.children.iter().rev() {
            if let Some(h) = self.hit_node(c, x, y) {
                return Some(h);
            }
        }
        inside.then_some(id)
    }

    /// Nearest ancestor-or-self with an action.
    pub fn action_at(&self, x: f32, y: f32) -> Option<(NodeId, &str)> {
        let mut cur = self.hit(x, y);
        while let Some(id) = cur {
            if let Some(a) = &self.nodes[id].el.action {
                return Some((id, a));
            }
            cur = self.nodes[id].parent;
        }
        None
    }

    /// Nearest ancestor-or-self that scrolls.
    pub fn scroller_at(&self, x: f32, y: f32) -> Option<NodeId> {
        let mut cur = self.hit(x, y);
        while let Some(id) = cur {
            if self.nodes[id].style.scroll {
                return Some(id);
            }
            cur = self.nodes[id].parent;
        }
        None
    }

    pub fn max_scroll(&self, id: NodeId) -> f32 {
        (self.nodes[id].content_h - self.content_box(id).h).max(0.0)
    }

    pub fn content_rect(&self, id: NodeId) -> Rect {
        self.content_box(id)
    }
}
