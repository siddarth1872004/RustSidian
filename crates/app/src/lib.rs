//! Rustsidian application library: state, commands, view construction and headless rendering.

pub mod anim;
pub mod canvasview;
pub mod cli;
pub mod clipboard;
pub mod editing;
pub mod graphview;
pub mod icon;
pub mod icons;
pub mod media;
pub mod menu;
pub mod plugins;
pub mod properties;
pub mod recent;
pub mod rich;
pub mod settings;
pub mod shell;
pub mod view;
pub mod welcome;

pub use shell::{App, Key};

use rustsidian_ui::{Dom, Fonts, Pixmap};
use std::time::Instant;
use theme_engine::Env;

/// One rendered frame and the information needed to map input back onto it.
pub struct Frame {
    pub dom: Dom,
    pub info: view::ViewInfo,
}

/// Build, style, lay out and paint the window into `pixmap`.
pub fn render(app: &mut App, fonts: &mut Fonts, pixmap: &mut Pixmap, scale: f32) -> Frame {
    let t = Instant::now();
    app.pump_plugins();
    app.refresh_derived();
    app.animate_changes();
    app.window_w = pixmap.width() as f32 / scale;
    let (mut dom, info) = view::build(app, pixmap.height() as f32 / scale);
    let env = Env { dark: app.dark, reduced_motion: false };
    dom.style(&app.theme, env);
    dom.layout(fonts, pixmap.width() as f32 / scale, pixmap.height() as f32 / scale);
    icons::paint(&mut dom);
    view::add_carets(app, &mut dom, &info, fonts);
    view::add_graph(app, &mut dom, &info);
    canvasview::add_canvas(app, &mut dom, &info);
    rich::paint(&mut dom, &info.rich, fonts);
    let scene = dom.paint();
    let clear = dom.nodes[0].style.background;
    rustsidian_ui::render(&scene, fonts, pixmap, scale, clear);
    app.last_frame_us = t.elapsed().as_micros() as u64;
    Frame { dom, info }
}

impl App {
    /// Left click at window coordinates.
    pub fn click(&mut self, frame: &Frame, fonts: &mut Fonts, x: f32, y: f32, shift: bool) {
        if self.menu_click(frame, x, y) {
            return;
        }
        if let Some((_, action)) = frame.dom.action_at(x, y) {
            let action = action.to_string();
            if !action.starts_with("prop:") && !action.starts_with("settings:") {
                self.commit_prop_edit();
            }
            return self.run(&action);
        }
        if self.prompt.is_some() {
            return self.run("prompt:dismiss");
        }
        if let (Some(rect), Some(g)) = (graph_rect(frame), &mut self.graph_view) {
            if rect.contains(x, y) {
                g.press(&rect, x, y);
            }
            return;
        }
        if self.canvas_click(frame, x, y) {
            return;
        }
        if let Some(off) = view::offset_at(&frame.dom, &frame.info, fonts, x, y) {
            self.focus = shell::Focus::Editor;
            if let Some(t) = self.tab_mut() {
                let anchor = if shift { t.editor.buf.sels[0].anchor } else { off };
                t.editor.buf.set_sels(vec![editor_core::Sel::new(anchor, off)]);
            }
        }
    }

    /// Extend the selection while dragging with the button held.
    pub fn drag(&mut self, frame: &Frame, fonts: &mut Fonts, x: f32, y: f32) {
        if let (Some(rect), Some(g)) = (graph_rect(frame), &mut self.graph_view) {
            return g.drag(&rect, x, y);
        }
        if self.canvas_drag(frame, x, y) {
            return;
        }
        if let Some(off) = view::offset_at(&frame.dom, &frame.info, fonts, x, y) {
            if let Some(t) = self.tab_mut() {
                let anchor = t.editor.buf.sels[0].anchor;
                t.editor.buf.set_sels(vec![editor_core::Sel::new(anchor, off)]);
            }
        }
    }

    /// Mouse wheel. `lines` is positive when scrolling down.
    pub fn wheel(&mut self, frame: &Frame, x: f32, y: f32, lines: f32) {
        self.menu = None;
        if let (Some(rect), Some(g)) = (graph_rect(frame), &mut self.graph_view) {
            if rect.contains(x, y) {
                return g.wheel(&rect, x, y, lines / 3.0);
            }
        }
        if self.canvas_wheel(frame, x, y, lines) {
            return;
        }
        let Some(node) = frame.dom.scroller_at(x, y) else { return };
        if Some(node) == frame.info.editor {
            return self.scroll_editor(lines.round() as isize);
        }
        if let Some(id) = frame.dom.nodes[node].el.id.clone() {
            let cur = frame.dom.nodes[node].scroll_y;
            self.scrolls.insert(id, (cur + lines * 40.0).clamp(0.0, frame.dom.max_scroll(node)));
        }
    }

    /// Middle click: closes the tab under the pointer.
    pub fn middle_click(&mut self, frame: &Frame, x: f32, y: f32) {
        self.menu = None;
        let Some((_, action)) = frame.dom.action_at(x, y) else { return };
        match action {
            "graph:focus" => self.run("graph:close"),
            "canvas:focus" => self.run("canvas:close"),
            a => {
                if let Some(i) = a.strip_prefix("tab:").or_else(|| a.strip_prefix("close-tab:")) {
                    let cmd = format!("close-tab:{i}");
                    self.run(&cmd);
                }
            }
        }
    }

    /// The left button was released.
    pub fn release(&mut self) {
        if let Some(kind) = self.graph_view.as_mut().and_then(graphview::GraphView::release) {
            self.graph_clicked(kind);
        }
        self.canvas_release();
    }

    /// The pointer moved with no button held. Returns true when the frame must be redrawn.
    pub fn hover(&mut self, frame: &Frame, x: f32, y: f32) -> bool {
        let action = frame.dom.action_at(x, y).map(|(_, a)| a.to_string());
        let changed = action != self.hover_action;
        self.hover_action = action;
        let graph = match (graph_rect(frame), &mut self.graph_view) {
            (Some(rect), Some(g)) => g.hover_at(&rect, x, y),
            _ => false,
        };
        graph || changed
    }
}

/// The graph pane of a frame, when the graph is showing.
fn graph_rect(frame: &Frame) -> Option<rustsidian_ui::Rect> {
    frame.info.graph.map(|n| frame.dom.nodes[n].rect)
}
