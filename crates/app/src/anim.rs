//! Short interface animations. Each is a named clock; the view reads its eased progress.
//! Disabled by default, so headless rendering (tests, screenshots) always shows the end state.

use std::collections::HashMap;
use std::time::Instant;

pub const FAST_MS: f32 = 140.0;
pub const SLOW_MS: f32 = 200.0;

#[derive(Default)]
pub struct Anims {
    pub enabled: bool,
    clocks: HashMap<&'static str, (Instant, f32)>,
}

impl Anims {
    /// Start (or restart) an animation that runs for `ms` milliseconds.
    pub fn start(&mut self, name: &'static str, ms: f32) {
        if self.enabled {
            self.clocks.insert(name, (Instant::now(), ms));
        }
    }

    /// Eased progress from 0 to 1. An animation that is not running reads as finished.
    pub fn t(&self, name: &str) -> f32 {
        let Some((start, ms)) = self.clocks.get(name) else { return 1.0 };
        let x = (start.elapsed().as_secs_f32() * 1000.0 / ms).clamp(0.0, 1.0);
        // Ease-out cubic.
        1.0 - (1.0 - x).powi(3)
    }

    /// Progress toward an on/off state: rises to 1 when `on`, falls to 0 otherwise.
    pub fn toward(&self, name: &str, on: bool) -> f32 {
        let t = self.t(name);
        if on {
            t
        } else {
            1.0 - t
        }
    }

    /// Drop finished clocks; true while any animation is still running.
    pub fn tick(&mut self) -> bool {
        self.clocks.retain(|_, (start, ms)| start.elapsed().as_secs_f32() * 1000.0 < *ms);
        !self.clocks.is_empty()
    }

    pub fn active(&self) -> bool {
        self.clocks.values().any(|(start, ms)| start.elapsed().as_secs_f32() * 1000.0 < *ms)
    }
}

/// What the last frame showed, to notice changes that should animate.
#[derive(Default, Clone, PartialEq)]
pub struct Seen {
    pub prompt: bool,
    pub menu: bool,
    pub settings: bool,
    pub notice: Option<String>,
    pub left: bool,
    pub right: bool,
    pub view: String,
}

/// How long a notice stays before it fades away by itself.
const NOTICE_MS: u64 = 4500;

impl crate::App {
    /// Compare with the last frame and start the animations for what changed.
    pub fn animate_changes(&mut self) {
        self.anims.tick();
        let now = Seen {
            prompt: self.prompt.is_some(),
            menu: self.menu.is_some(),
            settings: self.settings.is_some(),
            notice: self.notice.clone(),
            left: self.left_open,
            right: self.right_open,
            view: if self.show_graph {
                "graph:".to_string()
            } else if self.canvas_active() {
                "canvas:".to_string()
            } else {
                self.tab().map(|t| t.path.clone()).unwrap_or_default()
            },
        };
        let Some(old) = self.seen.replace(now.clone()) else { return };
        if now.prompt && !old.prompt {
            self.anims.start("prompt", FAST_MS);
        }
        if now.menu && !old.menu {
            self.anims.start("menu", FAST_MS);
        }
        if now.settings && !old.settings {
            self.anims.start("settings", FAST_MS);
        }
        if now.notice != old.notice {
            self.notice_since = now.notice.is_some().then(std::time::Instant::now);
            if now.notice.is_some() {
                self.anims.start("notice", SLOW_MS);
            }
        }
        if now.left != old.left {
            self.anims.start("left", SLOW_MS);
        }
        if now.right != old.right {
            self.anims.start("right", SLOW_MS);
        }
        if now.view != old.view {
            self.anims.start("view", FAST_MS);
        }
    }

    /// Milliseconds until the current notice should disappear; `Some(0)` when it is due.
    pub fn notice_due_in(&self) -> Option<u64> {
        let since = self.notice_since?;
        self.notice.as_ref()?;
        Some(NOTICE_MS.saturating_sub(since.elapsed().as_millis() as u64))
    }
}
