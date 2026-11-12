//! UI toolkit for Rustsidian: a CSS-styled retained element tree rendered by a software
//! rasterizer. Window integration lives in the `app` crate; everything here is headless and can
//! render to a pixel buffer for tests.

pub mod dom;
pub mod scene;
pub mod text;

pub use dom::{Dom, El, NodeId, Style};
pub use scene::{render, Cmd, Image, Rect, Scene};
pub use text::{Fonts, Span, TextStyle};
pub use theme_engine::{Color, Env, Theme};
pub use tiny_skia::{Color as SkColor, Pixmap};

/// Style, lay out and paint a tree into a new pixmap. Convenience for tests and screenshots.
pub fn render_to_pixmap(
    dom: &mut Dom,
    theme: &Theme,
    env: Env,
    fonts: &mut Fonts,
    w: u32,
    h: u32,
    scale: f32,
) -> Option<Pixmap> {
    dom.style(theme, env);
    dom.layout(fonts, w as f32 / scale, h as f32 / scale);
    let scene = dom.paint();
    let mut pm = Pixmap::new(w, h)?;
    let clear = dom.nodes[0].style.background;
    render(&scene, fonts, &mut pm, scale, clear);
    Some(pm)
}
