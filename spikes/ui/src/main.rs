use cosmic_text::{Attrs, Buffer, Color, FontSystem, Metrics, Shaping, SwashCache};
use std::time::Instant;
use tiny_skia::{Paint, Pixmap, Rect, Transform};

fn main() {
    let (w, h) = (1920u32, 1080u32);
    let t = Instant::now();
    let mut fs = FontSystem::new();
    let font_init = t.elapsed();
    let mut cache = SwashCache::new();
    let mut pm = Pixmap::new(w, h).unwrap();
    let text = "Rustwalnut — the quick brown fox jumps over the lazy dog. [[Wikilink]] **bold** #tag ﬁ → λ\n".repeat(60);
    let mut buf = Buffer::new(&mut fs, Metrics::new(15.0, 22.0));
    buf.set_size(Some(w as f32 - 40.0), Some(h as f32));
    let t = Instant::now();
    buf.set_text(&text, &Attrs::new(), Shaping::Advanced, None);
    buf.shape_until_scroll(&mut fs, false);
    let shape = t.elapsed();
    let mut frame = |pm: &mut Pixmap, fs: &mut FontSystem, cache: &mut SwashCache| {
        pm.fill(tiny_skia::Color::from_rgba8(30, 30, 30, 255));
        let mut paint = Paint::default();
        paint.anti_alias = false;
        for i in 0..200 {
            paint.set_color_rgba8(40 + (i % 50) as u8, 44, 52, 255);
            let r = Rect::from_xywh((i % 20) as f32 * 96.0, (i / 20) as f32 * 108.0, 90.0, 100.0).unwrap();
            pm.fill_rect(r, &paint, Transform::identity(), None);
        }
        let stride = pm.width() as usize;
        let (pw, ph) = (pm.width() as i32, pm.height() as i32);
        let data = pm.data_mut();
        buf.draw(fs, cache, Color::rgb(220, 220, 220), |x, y, gw, gh, c| {
            let a = c.a() as u32;
            if a == 0 { return; }
            for yy in y..y + gh as i32 {
                for xx in x..x + gw as i32 {
                    let (px, py) = (xx + 20, yy + 10);
                    if px < 0 || py < 0 || px >= pw || py >= ph { continue; }
                    let o = (py as usize * stride + px as usize) * 4;
                    for (k, v) in [c.r(), c.g(), c.b()].into_iter().enumerate() {
                        data[o + k] = ((v as u32 * a + data[o + k] as u32 * (255 - a)) / 255) as u8;
                    }
                }
            }
        });
    };
    let t = Instant::now();
    frame(&mut pm, &mut fs, &mut cache);
    let first = t.elapsed();
    let t = Instant::now();
    for _ in 0..20 { frame(&mut pm, &mut fs, &mut cache); }
    let warm = t.elapsed() / 20;
    pm.save_png(std::env::args().nth(1).unwrap_or("spike-ui.png".into())).unwrap();
    println!("font system init {font_init:?}; shape 60 lines {shape:?}; first full 1080p frame {first:?}; warm full frame {warm:?}");
}
