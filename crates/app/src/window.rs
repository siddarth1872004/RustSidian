//! The window: winit for events, softbuffer to present the software-rendered frame.
//! Fully event-driven: the loop sleeps until input, a file change or a deadline (autosave,
//! a notice timing out), and redraws at display rate only while something is animating.

use rustsidian_app::welcome::Welcome;
use rustsidian_app::{recent, App, Frame, Key};
use rustsidian_ui::{Dom, Fonts, Pixmap};
use std::num::NonZeroU32;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use vault_core::fsio::now_ms;
use vault_core::watch::VaultWatcher;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key as WKey, ModifiersState, NamedKey};
use winit::window::{CursorIcon, Icon, Window, WindowId};

/// Changed paths from a vault watcher, tagged with the generation of the vault it watches.
type Changed = (u64, Vec<String>);

struct Win {
    /// The open vault. `None` while the welcome screen is showing.
    app: Option<App>,
    welcome: Option<Welcome>,
    welcome_dom: Option<Dom>,
    fonts: Fonts,
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    pixmap: Option<Pixmap>,
    frame: Option<Frame>,
    mods: ModifiersState,
    mouse: (f32, f32),
    dragging: bool,
    animating: bool,
    pointer: bool,
    last_anim: Instant,
    started: Instant,
    profile: bool,
    first_frame_done: bool,
    proxy: EventLoopProxy<Changed>,
    /// Counts opened vaults; a watcher stops when its generation is no longer current.
    generation: Arc<AtomicU64>,
    watcher: Option<std::thread::Thread>,
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
        if let Some(welcome) = &mut self.welcome {
            self.welcome_dom = Some(welcome.render(&mut self.fonts, pixmap, scale));
        } else if let Some(app) = &mut self.app {
            self.frame = Some(rustsidian_app::render(app, &mut self.fonts, pixmap, scale));
        } else {
            return;
        }
        if surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buffer) = surface.buffer_mut() else { return };
        for (dst, px) in buffer.iter_mut().zip(pixmap.data().as_chunks::<4>().0) {
            *dst = (u32::from(px[0]) << 16) | (u32::from(px[1]) << 8) | u32::from(px[2]);
        }
        let _ = buffer.present();
        let Some(app) = &mut self.app else { return };
        if !self.first_frame_done {
            self.first_frame_done = true;
            // The link graph (backlinks) is computed after the first frame is on screen.
            app.defer_graph = false;
            window.request_redraw();
            if self.profile {
                eprintln!(
                    "[profile] first frame presented {:?} after process start (frame {:.2} ms)",
                    self.started.elapsed(),
                    app.last_frame_us as f64 / 1000.0
                );
            }
        }
    }

    fn request_redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn title(&self) -> String {
        match &self.app {
            Some(app) => format!(
                "{} - Rustsidian",
                app.store.root.file_name().and_then(|n| n.to_str()).unwrap_or("Vault")
            ),
            None => "Rustsidian".to_string(),
        }
    }

    fn stop_watcher(&mut self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(t) = self.watcher.take() {
            t.unpark();
        }
    }

    /// Open a vault in this window, replacing the welcome screen or the current vault.
    fn open_vault(&mut self, vault: &Path, note: Option<&str>) -> Result<(), String> {
        let app = load(vault, note, self.profile, self.started)?;
        self.install(app);
        Ok(())
    }

    /// Make a loaded vault the one this window shows, and watch its files.
    fn install(&mut self, app: App) {
        self.stop_watcher();
        // Registering recursive watches walks the whole vault, so it runs off the start-up path.
        let proxy = self.proxy.clone();
        let root = app.store.root.clone();
        let current = self.generation.clone();
        let mine = current.load(Ordering::SeqCst);
        let handle = std::thread::spawn(move || {
            let watcher = VaultWatcher::start(&root, Duration::from_millis(150), move |paths| {
                let _ = proxy.send_event((mine, paths));
            });
            // Keep the watcher alive while this vault is open; the thread sleeps, it does not poll.
            while watcher.is_ok() && current.load(Ordering::SeqCst) == mine {
                std::thread::park();
            }
        });
        self.watcher = Some(handle.thread().clone());
        self.app = Some(app);
        self.welcome = None;
        self.welcome_dom = None;
        self.frame = None;
        self.first_frame_done = false;
        if let Some(w) = &self.window {
            w.set_title(&self.title());
        }
    }

    /// Leave the current vault (if any) and show the vault chooser.
    fn show_welcome(&mut self) {
        if let Some(app) = &mut self.app {
            app.save_all();
        }
        self.stop_watcher();
        self.app = None;
        self.frame = None;
        let mut welcome = Welcome::new();
        welcome.intro();
        self.welcome = Some(welcome);
        if let Some(w) = &self.window {
            w.set_title(&self.title());
        }
    }

    /// A vault was chosen on the welcome screen.
    fn chosen(&mut self, vault: Option<std::path::PathBuf>) {
        let Some(vault) = vault else { return };
        let note = self.welcome.as_mut().and_then(|w| w.open_note.take());
        if let Err(e) = self.open_vault(&vault, note.as_deref()) {
            if let Some(w) = &mut self.welcome {
                w.error = Some(e);
            }
        }
    }

    /// Show a hand over things that can be clicked.
    fn set_pointer(&mut self, pointer: bool) {
        if pointer != self.pointer {
            self.pointer = pointer;
            if let Some(w) = &self.window {
                w.set_cursor(if pointer { CursorIcon::Pointer } else { CursorIcon::Default });
            }
        }
    }

    /// After input reached the app: follow its requests to quit or to switch vaults.
    fn after_input(&mut self, el: &ActiveEventLoop) {
        let Some(app) = &mut self.app else { return };
        if app.quit {
            el.exit();
        } else if app.switch_vault {
            app.switch_vault = false;
            self.show_welcome();
        }
    }

    fn welcome_event(&mut self, el: &ActiveEventLoop, event: WindowEvent) {
        let scale = self.window.as_ref().map_or(1.0, |w| w.scale_factor()) as f32;
        let Some(welcome) = &mut self.welcome else { return };
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Ime(Ime::Commit(text)) => welcome.text_input(&text),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let ctrl = self.mods.control_key() || self.mods.super_key();
                match &event.logical_key {
                    WKey::Character(c) if ctrl && c.eq_ignore_ascii_case("q") => return el.exit(),
                    WKey::Named(n) if named(n).is_some_and(|n| n != " ") => {
                        let vault = welcome.key(named(n).unwrap_or_default());
                        self.chosen(vault);
                    }
                    _ if !ctrl && !self.mods.alt_key() => {
                        if let Some(text) = &event.text {
                            welcome.text_input(text);
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse = (position.x as f32 / scale, position.y as f32 / scale);
                if let Some(dom) = &self.welcome_dom {
                    let changed = welcome.hover(dom, self.mouse.0, self.mouse.1);
                    let pointer = welcome.hover_action.is_some();
                    self.set_pointer(pointer);
                    if !changed {
                        return;
                    }
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if let Some(dom) = &self.welcome_dom {
                    let vault = welcome.click(dom, self.mouse.0, self.mouse.1);
                    self.chosen(vault);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * 3.0,
                    MouseScrollDelta::PixelDelta(p) => -(p.y as f32) / 20.0,
                };
                if let Some(dom) = &self.welcome_dom {
                    welcome.wheel(dom, self.mouse.0, self.mouse.1, lines);
                }
            }
            _ => return,
        }
        self.request_redraw();
    }
}

impl ApplicationHandler<Changed> for Win {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let icon = Icon::from_rgba(rustsidian_app::icon::rgba(64), 64, 64).ok();
        let attrs = Window::default_attributes()
            .with_title(self.title())
            .with_window_icon(icon)
            .with_inner_size(LogicalSize::new(1280.0, 800.0));
        let window = match el.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("rustsidian: cannot create a window: {e}");
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
                eprintln!("rustsidian: cannot create a drawing surface: {e}");
                el.exit();
                return;
            }
        }
        self.window = Some(window);
    }

    fn user_event(&mut self, _: &ActiveEventLoop, (generation, paths): Changed) {
        if generation != self.generation.load(Ordering::SeqCst) {
            return;
        }
        if let Some(app) = &mut self.app {
            app.files_changed(&paths);
            self.request_redraw();
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::RedrawRequested => return self.redraw(),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => return self.request_redraw(),
            WindowEvent::ModifiersChanged(m) => return self.mods = m.state(),
            _ => {}
        }
        if self.welcome.is_some() {
            return self.welcome_event(el, event);
        }
        let scale = self.window.as_ref().map_or(1.0, |w| w.scale_factor()) as f32;
        let Some(app) = &mut self.app else { return };
        match event {
            WindowEvent::CloseRequested => {
                app.save_all();
                el.exit();
            }
            WindowEvent::Focused(false) => app.save_all(),
            WindowEvent::Ime(Ime::Commit(text)) => {
                app.text_input(&text);
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
                        consumed = app.key(key);
                    }
                }
                if !consumed && !self.mods.control_key() && !self.mods.alt_key() {
                    if let Some(text) = &event.text {
                        app.text_input(text);
                    }
                }
                self.after_input(el);
                self.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse = (position.x as f32 / scale, position.y as f32 / scale);
                let Some(frame) = &self.frame else { return };
                if self.dragging {
                    app.drag(frame, &mut self.fonts, self.mouse.0, self.mouse.1);
                    self.request_redraw();
                } else {
                    let changed = app.hover(frame, self.mouse.0, self.mouse.1);
                    let pointer = app.hover_action.is_some();
                    self.set_pointer(pointer);
                    if changed {
                        self.request_redraw();
                    }
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                self.dragging = state == ElementState::Pressed;
                if self.dragging {
                    if let Some(frame) = &self.frame {
                        app.click(frame, &mut self.fonts, self.mouse.0, self.mouse.1, self.mods.shift_key());
                    }
                } else {
                    app.release();
                }
                self.after_input(el);
                self.request_redraw();
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Right, .. } => {
                if let Some(frame) = &self.frame {
                    app.context_menu(frame, &mut self.fonts, self.mouse.0, self.mouse.1);
                    self.request_redraw();
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Middle, .. } => {
                if let Some(frame) = &self.frame {
                    app.middle_click(frame, self.mouse.0, self.mouse.1);
                    self.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * 3.0,
                    MouseScrollDelta::PixelDelta(p) => -(p.y as f32) / 20.0,
                };
                if let Some(frame) = &self.frame {
                    app.wheel(frame, self.mouse.0, self.mouse.1, lines);
                    self.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let frame_due = self.last_anim.elapsed().as_millis() as u64 >= 16;
        let Some(app) = &mut self.app else {
            // Welcome screen: only its own animations need frames.
            let animating = self.welcome.as_ref().is_some_and(|w| w.anims.active());
            if (animating && frame_due) || (self.animating && !animating) {
                self.last_anim = Instant::now();
                self.request_redraw();
            }
            self.animating = animating;
            return el.set_control_flow(if animating {
                ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(16))
            } else {
                ControlFlow::Wait
            });
        };
        // Sleep until the next deadline (outline refresh, autosave, notice), or indefinitely.
        let now = now_ms();
        let mut redraw = app.derived_due_in(now) == Some(0);
        if app.autosave_due_in(now) == Some(0) {
            app.save_all();
            redraw = true;
        }
        if app.notice_due_in() == Some(0) {
            app.notice = None;
            redraw = true;
        }
        let now = now_ms();
        let next = [app.derived_due_in(now), app.autosave_due_in(now), app.notice_due_in()]
            .into_iter()
            .flatten()
            .filter(|&ms| ms > 0)
            .min();
        // While the graph layout or an interface animation is moving, redraw at display rate;
        // both stop by themselves.
        let animating = app.graph_animating() || app.anims.active();
        if (animating && frame_due) || (self.animating && !animating) {
            self.last_anim = Instant::now();
            redraw = true;
        }
        self.animating = animating;
        if redraw {
            self.request_redraw();
        }
        let next = if animating { Some(next.map_or(16, |ms| ms.min(16)).max(1)) } else { next };
        match next {
            Some(ms) => {
                el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(ms)))
            }
            None => el.set_control_flow(ControlFlow::Wait),
        }
    }
}

/// Open a vault and prepare it for a window.
fn load(vault: &Path, note: Option<&str>, profile: bool, started: Instant) -> Result<App, String> {
    let mut app = App::open(vault).map_err(|e| format!("Cannot open {}: {e}", vault.display()))?;
    if profile {
        eprintln!("[profile] vault opened in {:?} ({} files)", started.elapsed(), app.store.index.len());
    }
    app.defer_graph = true;
    app.anims.enabled = true;
    app.system_clipboard = true;
    if let Some(n) = note {
        app.open_note(n);
    }
    recent::remember(vault);
    Ok(app)
}

/// Run the window. With no vault given, the most recent vault is reopened; when there is none,
/// the welcome screen asks for one.
pub fn run(vault: Option<&Path>, note: Option<&str>, profile: bool) -> i32 {
    let started = Instant::now();
    // Fonts load on a worker thread while the vault index loads on this one.
    let fonts = std::thread::spawn(Fonts::new);
    let initial = match vault {
        Some(vault) => match load(vault, note, profile, started) {
            Ok(app) => Some(app),
            Err(e) => {
                eprintln!("rustsidian: {e}");
                return 1;
            }
        },
        None => recent::load().into_iter().next().and_then(|v| load(&v, None, profile, started).ok()),
    };
    let event_loop = match EventLoop::<Changed>::with_user_event().build() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("rustsidian: no display available: {e}");
            return 1;
        }
    };
    let Ok(fonts) = fonts.join() else { return 1 };
    if profile {
        eprintln!("[profile] fonts ready at {:?} ({} faces)", started.elapsed(), fonts.font_count());
    }
    let mut win = Win {
        app: None,
        welcome: None,
        welcome_dom: None,
        fonts,
        window: None,
        surface: None,
        pixmap: None,
        frame: None,
        mods: ModifiersState::empty(),
        mouse: (0.0, 0.0),
        dragging: false,
        animating: false,
        pointer: false,
        last_anim: started,
        started,
        profile,
        first_frame_done: false,
        proxy: event_loop.create_proxy(),
        generation: Arc::new(AtomicU64::new(0)),
        watcher: None,
    };
    match initial {
        Some(app) => win.install(app),
        None => win.show_welcome(),
    }
    match event_loop.run_app(&mut win) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("rustsidian: {e}");
            1
        }
    }
}
