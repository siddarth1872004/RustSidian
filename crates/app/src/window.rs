//! The window: winit for events, softbuffer to present the software-rendered frame.
//! Fully event-driven: the loop sleeps until input, a file change or the autosave deadline.

use std::num::NonZeroU32;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use vault_core::fsio::now_ms;
use vault_core::watch::VaultWatcher;
use walnut_app::{App, Frame, Key};
use walnut_ui::{Fonts, Pixmap};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key as WKey, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

struct Win {
    app: App,
    fonts: Fonts,
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    pixmap: Option<Pixmap>,
    frame: Option<Frame>,
    mods: ModifiersState,
    mouse: (f32, f32),
    dragging: bool,
    animating: bool,
    last_anim: Instant,
    started: Instant,
    profile: bool,
    first_frame_done: bool,
}

fn named(k: &NamedKey) -> Option<&'static str> {
    Some(match k {
        NamedKey::Enter => "Enter",
        NamedKey::Escape => "Escape",
        NamedKey::Tab => "Tab",
        NamedKey::Backspace => "Backspace",
        NamedKey::Delete => "Delete",
        NamedKey::ArrowLeft => "ArrowLeft",
        NamedKey::ArrowRight => "ArrowRight",
        NamedKey::ArrowUp => "ArrowUp",
        NamedKey::ArrowDown => "ArrowDown",
        NamedKey::Home => "Home",
        NamedKey::End => "End",
        NamedKey::PageUp => "PageUp",
        NamedKey::PageDown => "PageDown",
        NamedKey::Space => " ",
        NamedKey::F12 => "F12",
        _ => return None,
    })
}

impl Win {
    fn redraw(&mut self) {
        let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else { return };
        let size = window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        if self.pixmap.as_ref().is_none_or(|p| p.width() != size.width || p.height() != size.height) {
            self.pixmap = Pixmap::new(size.width, size.height);
        }
        let Some(pixmap) = &mut self.pixmap else { return };
        let scale = window.scale_factor() as f32;
        self.frame = Some(walnut_app::render(&mut self.app, &mut self.fonts, pixmap, scale));
        if surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buffer) = surface.buffer_mut() else { return };
        for (dst, px) in buffer.iter_mut().zip(pixmap.data().as_chunks::<4>().0) {
            *dst = (u32::from(px[0]) << 16) | (u32::from(px[1]) << 8) | u32::from(px[2]);
        }
        let _ = buffer.present();
        if !self.first_frame_done {
            self.first_frame_done = true;
            // The link graph (backlinks) is computed after the first frame is on screen.
            self.app.defer_graph = false;
            window.request_redraw();
            if self.profile {
                eprintln!(
                    "[profile] first frame presented {:?} after process start (frame {:.2} ms)",
                    self.started.elapsed(),
                    self.app.last_frame_us as f64 / 1000.0
                );
            }
        }
    }

    fn request_redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}

impl ApplicationHandler<Vec<String>> for Win {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let title = format!(
            "{} - Rustwalnut",
            self.app.store.root.file_name().and_then(|n| n.to_str()).unwrap_or("Vault")
        );
        let attrs =
            Window::default_attributes().with_title(title).with_inner_size(LogicalSize::new(1280.0, 800.0));
        let window = match el.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("walnut: cannot create a window: {e}");
                el.exit();
                return;
            }
        };
        window.set_ime_allowed(true);
        let surface = softbuffer::Context::new(window.clone())
            .and_then(|cx| softbuffer::Surface::new(&cx, window.clone()));
        match surface {
            Ok(s) => self.surface = Some(s),
            Err(e) => {
                eprintln!("walnut: cannot create a drawing surface: {e}");
                el.exit();
                return;
            }
        }
        self.window = Some(window);
    }

    fn user_event(&mut self, _: &ActiveEventLoop, paths: Vec<String>) {
        self.app.files_changed(&paths);
        self.request_redraw();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.app.save_all();
                el.exit();
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => self.request_redraw(),
            WindowEvent::Focused(false) => self.app.save_all(),
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::Ime(Ime::Commit(text)) => {
                self.app.text_input(&text);
                self.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let name = match &event.logical_key {
                    WKey::Named(n) => named(n).map(str::to_string),
                    WKey::Character(c) => Some(c.to_string()),
                    _ => None,
                };
                let mut consumed = false;
                if let Some(name) = name {
                    let mut key = Key::new(&name);
                    key.ctrl = self.mods.control_key() || self.mods.super_key();
                    key.shift = self.mods.shift_key();
                    key.alt = self.mods.alt_key();
                    // Plain characters are text, not shortcuts, unless a modifier is held.
                    let plain_char = name.chars().count() == 1 && !key.ctrl && !key.alt;
                    if !plain_char {
                        consumed = self.app.key(key);
                    }
                }
                if !consumed && !self.mods.control_key() && !self.mods.alt_key() {
                    if let Some(text) = &event.text {
                        self.app.text_input(text);
                    }
                }
                if self.app.quit {
                    el.exit();
                }
                self.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let scale = self.window.as_ref().map_or(1.0, |w| w.scale_factor()) as f32;
                self.mouse = (position.x as f32 / scale, position.y as f32 / scale);
                if self.dragging {
                    if let Some(frame) = &self.frame {
                        self.app.drag(frame, &mut self.fonts, self.mouse.0, self.mouse.1);
                        self.request_redraw();
                    }
                } else if let Some(frame) = &self.frame {
                    if self.app.hover(frame, self.mouse.0, self.mouse.1) {
                        self.request_redraw();
                    }
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                self.dragging = state == ElementState::Pressed;
                if self.dragging {
                    if let Some(frame) = &self.frame {
                        self.app.click(
                            frame,
                            &mut self.fonts,
                            self.mouse.0,
                            self.mouse.1,
                            self.mods.shift_key(),
                        );
                        self.request_redraw();
                    }
                } else {
                    self.app.release();
                    self.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * 3.0,
                    MouseScrollDelta::PixelDelta(p) => -(p.y as f32) / 20.0,
                };
                if let Some(frame) = &self.frame {
                    self.app.wheel(frame, self.mouse.0, self.mouse.1, lines);
                    self.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        // Sleep until the next deadline (outline refresh, autosave), or indefinitely.
        let now = now_ms();
        if self.app.derived_due_in(now) == Some(0) {
            self.request_redraw();
        }
        if self.app.autosave_due_in(now) == Some(0) {
            self.app.save_all();
            self.request_redraw();
        }
        let now = now_ms();
        let next = [self.app.derived_due_in(now), self.app.autosave_due_in(now)]
            .into_iter()
            .flatten()
            .filter(|&ms| ms > 0)
            .min();
        // While the graph layout is moving, redraw at display rate; it stops by itself.
        let animating = self.app.graph_animating();
        let since = self.last_anim.elapsed().as_millis() as u64;
        if (animating && since >= 16) || (self.animating && !animating) {
            self.last_anim = Instant::now();
            self.request_redraw();
        }
        self.animating = animating;
        let next = if animating { Some(next.map_or(16, |ms| ms.min(16)).max(1)) } else { next };
        match next {
            Some(ms) => {
                el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(ms)))
            }
            None => el.set_control_flow(ControlFlow::Wait),
        }
    }
}

pub fn run(vault: &Path, note: Option<&str>, profile: bool) -> i32 {
    let started = Instant::now();
    // Fonts load on a worker thread while the vault index loads on this one.
    let fonts = std::thread::spawn(Fonts::new);
    let mut app = match App::open(vault) {
        Ok(app) => app,
        Err(e) => {
            eprintln!("walnut: cannot open {}: {e}", vault.display());
            return 1;
        }
    };
    if profile {
        eprintln!("[profile] vault opened in {:?} ({} files)", started.elapsed(), app.store.index.len());
    }
    app.defer_graph = true;
    if let Some(n) = note {
        app.open_note(n);
    }
    let event_loop = match EventLoop::<Vec<String>>::with_user_event().build() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("walnut: no display available: {e}");
            return 1;
        }
    };
    // Registering recursive watches walks the whole vault, so it runs off the start-up path.
    let proxy = event_loop.create_proxy();
    let watch_root = vault.to_path_buf();
    std::thread::spawn(move || {
        let watcher = VaultWatcher::start(&watch_root, Duration::from_millis(150), move |paths| {
            let _ = proxy.send_event(paths);
        });
        // Keep the watcher alive for the life of the process; the thread sleeps, it does not poll.
        if watcher.is_ok() {
            loop {
                std::thread::park();
            }
        }
    });
    let Ok(fonts) = fonts.join() else { return 1 };
    if profile {
        eprintln!("[profile] fonts ready at {:?} ({} faces)", started.elapsed(), fonts.font_count());
    }
    let mut win = Win {
        app,
        fonts,
        window: None,
        surface: None,
        pixmap: None,
        frame: None,
        mods: ModifiersState::empty(),
        mouse: (0.0, 0.0),
        dragging: false,
        animating: false,
        last_anim: started,
        started,
        profile,
        first_frame_done: false,
    };
    match event_loop.run_app(&mut win) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("walnut: {e}");
            1
        }
    }
}
