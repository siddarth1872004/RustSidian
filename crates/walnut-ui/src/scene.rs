//! Display list and the software rasterizer (tiny-skia).

use crate::text::{Fonts, Span, TextStyle};
use rayon::prelude::*;
use std::sync::Arc;
use theme_engine::Color;
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};

/// A decoded image shared between frames. Two images are equal when they are the same pixmap.
#[derive(Clone)]
pub struct Image(pub Arc<Pixmap>);

impl Image {
    /// Decode a PNG, scaled down if needed so that neither side exceeds `max_side` pixels.
    pub fn decode_png(bytes: &[u8], max_side: u32) -> Option<Image> {
        let p = Pixmap::decode_png(bytes).ok()?;
        let side = p.width().max(p.height());
        if side <= max_side.max(1) {
            return Some(Image(Arc::new(p)));
        }
        let k = max_side.max(1) as f32 / side as f32;
        let (w, h) = (((p.width() as f32 * k) as u32).max(1), ((p.height() as f32 * k) as u32).max(1));
        let mut out = Pixmap::new(w, h)?;
        let paint =
            tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
        out.draw_pixmap(0, 0, p.as_ref(), &paint, Transform::from_scale(k, k), None);
        Some(Image(Arc::new(out)))
    }
    pub fn size(&self) -> (u32, u32) {
        (self.0.width(), self.0.height())
    }
}

impl PartialEq for Image {
    fn eq(&self, o: &Image) -> bool {
        Arc::ptr_eq(&self.0, &o.0)
    }
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Image({}x{})", self.0.width(), self.0.height())
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = (self.x + self.w).min(o.x + o.w);
        let b = (self.y + self.h).min(o.y + o.h);
        Rect { x, y, w: (r - x).max(0.0), h: (b - y).max(0.0) }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Cmd {
    Fill {
        rect: Rect,
        color: Color,
        radius: f32,
    },
    Text {
        x: f32,
        y: f32,
        width: Option<f32>,
        text: String,
        spans: Vec<Span>,
        style: TextStyle,
    },
    /// An image stretched to `rect`.
    Image {
        rect: Rect,
        image: Image,
    },
    /// Line segments `[x0, y0, x1, y1]` stroked in one color.
    Lines {
        segs: Vec<[f32; 4]>,
        width: f32,
        color: Color,
    },
    /// Filled circles `[cx, cy, radius]` in one color.
    Circles {
        items: Vec<[f32; 3]>,
        color: Color,
    },
    PushClip(Rect),
    PopClip,
}

/// Clip a segment to a rectangle (Liang-Barsky). `None` when it lies outside.
fn clip_segment(s: [f32; 4], r: &Rect) -> Option<[f32; 4]> {
    let (dx, dy) = (s[2] - s[0], s[3] - s[1]);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [(-dx, s[0] - r.x), (dx, r.x + r.w - s[0]), (-dy, s[1] - r.y), (dy, r.y + r.h - s[1])] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
            if t0 > t1 {
                return None;
            }
        }
    }
    Some([s[0] + dx * t0, s[1] + dy * t0, s[0] + dx * t1, s[1] + dy * t1])
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Scene {
    pub cmds: Vec<Cmd>,
}

fn rounded(r: Rect, radius: f32) -> Option<tiny_skia::Path> {
    let rad = radius.min(r.w / 2.0).min(r.h / 2.0);
    let (x, y, w, h) = (r.x, r.y, r.w, r.h);
    let k = rad * 0.5523;
    let mut p = PathBuilder::new();
    p.move_to(x + rad, y);
    p.line_to(x + w - rad, y);
    p.cubic_to(x + w - rad + k, y, x + w, y + rad - k, x + w, y + rad);
    p.line_to(x + w, y + h - rad);
    p.cubic_to(x + w, y + h - rad + k, x + w - rad + k, y + h, x + w - rad, y + h);
    p.line_to(x + rad, y + h);
    p.cubic_to(x + rad - k, y + h, x, y + h - rad + k, x, y + h - rad);
    p.line_to(x, y + rad);
    p.cubic_to(x, y + rad - k, x + rad - k, y, x + rad, y);
    p.close();
    p.finish()
}

/// A clip rectangle in device pixels, within the pixmap.
struct DevClip {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl DevClip {
    fn new(clip: &Rect, scale: f32, pixmap: &Pixmap) -> DevClip {
        DevClip {
            x0: ((clip.x * scale).floor() as i32).max(0),
            y0: ((clip.y * scale).floor() as i32).max(0),
            x1: (((clip.x + clip.w) * scale).ceil() as i32).min(pixmap.width() as i32),
            y1: (((clip.y + clip.h) * scale).ceil() as i32).min(pixmap.height() as i32),
        }
    }
}

/// Blend `c` over an opaque pixel with coverage `a` (0..=256).
#[inline]
fn blend(px: &mut [u8], c: [u32; 3], a: u32) {
    let ia = 256 - a;
    px[0] = ((c[0] * a + px[0] as u32 * ia) >> 8) as u8;
    px[1] = ((c[1] * a + px[1] as u32 * ia) >> 8) as u8;
    px[2] = ((c[2] * a + px[2] as u32 * ia) >> 8) as u8;
    px[3] = 255;
}

/// Run `f(band rows, band clip)` over horizontal bands of the clip, in parallel. Each band owns
/// its rows of the buffer, so shapes are rasterized without locking.
fn banded(
    data: &mut [u8],
    stride: usize,
    clip: &DevClip,
    work: usize,
    f: impl Fn(&mut [u8], &DevClip) + Sync,
) {
    if clip.x1 <= clip.x0 || clip.y1 <= clip.y0 {
        return;
    }
    let rows = (clip.y1 - clip.y0) as usize;
    // Small jobs are not worth waking the pool for.
    let bands = if work < 2000 { 1 } else { rayon::current_num_threads().clamp(1, rows.div_ceil(16)) };
    let band_h = rows.div_ceil(bands);
    let area = &mut data[clip.y0 as usize * stride * 4..clip.y1 as usize * stride * 4];
    let run = |(i, rows): (usize, &mut [u8])| {
        let y0 = clip.y0 + (i * band_h) as i32;
        f(rows, &DevClip { x0: clip.x0, x1: clip.x1, y0, y1: y0 + (rows.len() / (stride * 4)) as i32 });
    };
    if bands == 1 {
        run((0, area));
    } else {
        area.par_chunks_mut(band_h * stride * 4).enumerate().for_each(run);
    }
}

/// Anti-aliased one-pixel line (Xiaolin Wu) between device points, restricted to a band whose
/// first row is `rows[0..]`. `alpha` is 0..=256.
fn thin_line(rows: &mut [u8], stride: usize, band: &DevClip, s: [f32; 4], c: [u32; 3], alpha: f32) {
    let [mut x0, mut y0, mut x1, mut y1] = s;
    if y0.max(y1) + 1.0 < band.y0 as f32 || y0.min(y1) - 1.0 >= band.y1 as f32 {
        return;
    }
    let steep = (y1 - y0).abs() > (x1 - x0).abs();
    if steep {
        std::mem::swap(&mut x0, &mut y0);
        std::mem::swap(&mut x1, &mut y1);
    }
    if x0 > x1 {
        std::mem::swap(&mut x0, &mut x1);
        std::mem::swap(&mut y0, &mut y1);
    }
    let grad = if x1 - x0 < 1e-3 { 0.0 } else { (y1 - y0) / (x1 - x0) };
    let (xs, xe) = (x0.round() as i32, x1.round() as i32);
    let y = y0 + grad * (xs as f32 - x0) - 0.5;
    // `u` runs along the major axis, `v` across it.
    let (u0, u1, v0, v1) =
        if steep { (band.y0, band.y1, band.x0, band.x1) } else { (band.x0, band.x1, band.y0, band.y1) };
    let (mut from, mut to) = (xs.max(u0), (xe + 1).min(u1));
    if !steep && grad.abs() > 1e-6 {
        // Only the stretch of a shallow line that crosses this band.
        let at = |v: f32| xs as f32 + (v - y) / grad;
        let (a, b) = (at(v0 as f32 - 1.5), at(v1 as f32 + 0.5));
        from = from.max(a.min(b).floor() as i32);
        to = to.min(a.max(b).ceil() as i32 + 1);
    }
    if from >= to {
        return;
    }
    // 16.16 fixed point across the minor axis: no float rounding per pixel.
    let mut yf = ((y + grad * (from - xs) as f32) * 65536.0) as i32;
    let step = (grad * 65536.0) as i32;
    let alpha = alpha as u32;
    for u in from..to {
        let v = yf >> 16;
        let far = (alpha * ((yf & 0xffff) as u32 >> 8)) >> 8;
        yf = yf.wrapping_add(step);
        for (v, a) in [(v, alpha - far), (v + 1, far)] {
            if v >= v0 && v < v1 {
                let (px, py) = if steep { (v, u) } else { (u, v) };
                let o = ((py - band.y0) as usize * stride + px as usize) * 4;
                blend(&mut rows[o..o + 4], c, a);
            }
        }
    }
}

/// Anti-aliased filled circle in device pixels, restricted to a band.
fn disc(rows: &mut [u8], stride: usize, band: &DevClip, [cx, cy, r]: [f32; 3], c: [u32; 3], alpha: f32) {
    if r <= 0.0 {
        return;
    }
    let (x0, x1) = (((cx - r).floor() as i32).max(band.x0), ((cx + r).ceil() as i32 + 1).min(band.x1));
    let (y0, y1) = (((cy - r).floor() as i32).max(band.y0), ((cy + r).ceil() as i32 + 1).min(band.y1));
    let inner = (r - 0.5).max(0.0);
    let inner2 = inner * inner;
    for py in y0..y1 {
        let dy = py as f32 + 0.5 - cy;
        for px in x0..x1 {
            let dx = px as f32 + 0.5 - cx;
            let d2 = dx * dx + dy * dy;
            let cov = if d2 <= inner2 { 1.0 } else { (r + 0.5 - d2.sqrt()).clamp(0.0, 1.0) };
            if cov > 0.0 {
                let o = ((py - band.y0) as usize * stride + px as usize) * 4;
                blend(&mut rows[o..o + 4], c, (alpha * cov) as u32);
            }
        }
    }
}

/// Rasterize a scene. `scale` is the device pixel ratio; scene coordinates are logical pixels.
pub fn render(scene: &Scene, fonts: &mut Fonts, pixmap: &mut Pixmap, scale: f32, clear: Color) {
    pixmap.fill(tiny_skia::Color::from_rgba8(clear.r, clear.g, clear.b, 255));
    let full = Rect::new(0.0, 0.0, pixmap.width() as f32 / scale, pixmap.height() as f32 / scale);
    let mut clips = vec![full];
    for cmd in &scene.cmds {
        let clip = *clips.last().unwrap_or(&full);
        match cmd {
            Cmd::PushClip(r) => clips.push(clip.intersect(r)),
            Cmd::PopClip => {
                if clips.len() > 1 {
                    clips.pop();
                }
            }
            Cmd::Fill { rect, color, radius } => {
                if color.a == 0 {
                    continue;
                }
                let mut paint = Paint::default();
                paint.set_color_rgba8(color.r, color.g, color.b, color.a);
                if *radius > 0.5 {
                    // Rounded shapes are only clipped by rectangle rejection: draw when the
                    // shape is at least partly visible, clipped through a mask-free intersect.
                    let vis = rect.intersect(&clip);
                    if vis.w <= 0.0 || vis.h <= 0.0 {
                        continue;
                    }
                    paint.anti_alias = true;
                    if vis == *rect {
                        if let Some(path) = rounded(*rect, *radius) {
                            pixmap.fill_path(
                                &path,
                                &paint,
                                FillRule::Winding,
                                Transform::from_scale(scale, scale),
                                None,
                            );
                        }
                        continue;
                    }
                }
                let r = rect.intersect(&clip);
                paint.anti_alias = false;
                if let Some(tr) = tiny_skia::Rect::from_xywh(
                    (r.x * scale).round(),
                    (r.y * scale).round(),
                    (r.w * scale).round(),
                    (r.h * scale).round(),
                ) {
                    pixmap.fill_rect(tr, &paint, Transform::identity(), None);
                }
            }
            Cmd::Lines { segs, width, color } => {
                if color.a == 0 || clip.w <= 0.0 || clip.h <= 0.0 {
                    continue;
                }
                let dev = DevClip::new(&clip, scale, pixmap);
                if *width * scale <= 1.5 {
                    // Thin lines are drawn straight into the buffer: a path stroke of a whole
                    // graph costs seconds, this costs milliseconds.
                    let alpha = color.a as f32 * (256.0 / 255.0) * (*width * scale).min(1.0);
                    let rgb = [color.r as u32, color.g as u32, color.b as u32];
                    let stride = pixmap.width() as usize;
                    let lines: Vec<[f32; 4]> = segs
                        .iter()
                        .filter_map(|s| clip_segment(*s, &clip))
                        .map(|c| c.map(|v| v * scale))
                        .collect();
                    banded(pixmap.data_mut(), stride, &dev, lines.len(), |rows, band| {
                        for l in &lines {
                            thin_line(rows, stride, band, *l, rgb, alpha);
                        }
                    });
                    continue;
                }
                let mut pb = PathBuilder::new();
                for s in segs {
                    if let Some(c) = clip_segment(*s, &clip) {
                        pb.move_to(c[0], c[1]);
                        pb.line_to(c[2], c[3]);
                    }
                }
                let Some(path) = pb.finish() else { continue };
                let mut paint = Paint::default();
                paint.set_color_rgba8(color.r, color.g, color.b, color.a);
                paint.anti_alias = true;
                let stroke = tiny_skia::Stroke { width: *width, ..Default::default() };
                pixmap.stroke_path(&path, &paint, &stroke, Transform::from_scale(scale, scale), None);
            }
            Cmd::Circles { items, color } => {
                if color.a == 0 {
                    continue;
                }
                let dev = DevClip::new(&clip, scale, pixmap);
                let stride = pixmap.width() as usize;
                let alpha = color.a as f32 * (256.0 / 255.0);
                let rgb = [color.r as u32, color.g as u32, color.b as u32];
                banded(pixmap.data_mut(), stride, &dev, items.len(), |rows, band| {
                    for it in items {
                        let (y, r) = (it[1] * scale, it[2] * scale + 1.0);
                        if y + r >= band.y0 as f32 && y - r < band.y1 as f32 {
                            disc(rows, stride, band, it.map(|v| v * scale), rgb, alpha);
                        }
                    }
                });
            }
            Cmd::Image { rect, image } => {
                let vis = rect.intersect(&clip);
                let (iw, ih) = image.size();
                if vis.w <= 0.0 || vis.h <= 0.0 || iw == 0 || ih == 0 {
                    continue;
                }
                let mask = if vis == *rect {
                    None
                } else {
                    let clip_path =
                        tiny_skia::Rect::from_xywh(vis.x, vis.y, vis.w, vis.h).map(PathBuilder::from_rect);
                    let mut m = tiny_skia::Mask::new(pixmap.width(), pixmap.height());
                    if let (Some(m), Some(path)) = (m.as_mut(), clip_path) {
                        m.fill_path(&path, FillRule::Winding, false, Transform::from_scale(scale, scale));
                    }
                    m
                };
                let (sx, sy) = (rect.w * scale / iw as f32, rect.h * scale / ih as f32);
                let paint = tiny_skia::PixmapPaint {
                    quality: if (sx - 1.0).abs() < 0.01 && (sy - 1.0).abs() < 0.01 {
                        tiny_skia::FilterQuality::Nearest
                    } else {
                        tiny_skia::FilterQuality::Bilinear
                    },
                    ..Default::default()
                };
                let tf = Transform::from_scale(sx, sy).post_translate(rect.x * scale, rect.y * scale);
                pixmap.draw_pixmap(0, 0, image.0.as_ref().as_ref(), &paint, tf, mask.as_ref());
            }
            Cmd::Text { x, y, width, text, spans, style } => {
                let (pw, ph) = (pixmap.width() as i32, pixmap.height() as i32);
                let cx0 = (clip.x * scale).floor() as i32;
                let cy0 = (clip.y * scale).floor() as i32;
                let cx1 = (((clip.x + clip.w) * scale).ceil() as i32).min(pw);
                let cy1 = (((clip.y + clip.h) * scale).ceil() as i32).min(ph);
                if cx1 <= cx0 || cy1 <= cy0 {
                    continue;
                }
                let scaled = TextStyle {
                    size: style.size * scale,
                    line_height: style.line_height * scale,
                    ..style.clone()
                };
                let (ox, oy) = ((x * scale).round() as i32, (y * scale).round() as i32);
                let stride = pw as usize;
                let data = pixmap.data_mut();
                fonts.draw(text, spans, &scaled, width.map(|w| w * scale), |gx, gy, gw, gh, c| {
                    if c.a == 0 {
                        return;
                    }
                    let a = c.a as u32;
                    for py in (oy + gy).max(cy0).max(0)..(oy + gy + gh as i32).min(cy1) {
                        for px in (ox + gx).max(cx0).max(0)..(ox + gx + gw as i32).min(cx1) {
                            let o = (py as usize * stride + px as usize) * 4;
                            data[o] = ((c.r as u32 * a + data[o] as u32 * (255 - a)) / 255) as u8;
                            data[o + 1] = ((c.g as u32 * a + data[o + 1] as u32 * (255 - a)) / 255) as u8;
                            data[o + 2] = ((c.b as u32 * a + data[o + 2] as u32 * (255 - a)) / 255) as u8;
                            data[o + 3] = 255;
                        }
                    }
                });
            }
        }
    }
}
