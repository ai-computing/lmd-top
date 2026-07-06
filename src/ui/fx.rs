//! tachyonfx-based screen effects (pure Rust) — view-transition transitions + new-alert flash.
//! State (in-progress Effect) must persist across frames, so the UI loop owns it, not App (the data model).
//! Effects post-process the already-drawn buffer → applied over the area after widget render.

use super::theme::*;
use crate::app::{App, View};
use ratatui::layout::Rect;
use ratatui::Frame;
use std::time::Instant;
use tachyonfx::{fx, Duration as FxDuration, Effect, EffectRenderer, Interpolation, Shader};

pub struct FxState {
    enabled: bool,
    last: Instant,
    prev_view: Option<View>,
    prev_detail: bool,
    prev_flash: u64,
    body: Option<Effect>,  // view-switch / detail-entry transition (body area)
    flash: Option<Effect>, // new-alert flash (summary bar)
}

impl FxState {
    pub fn new() -> Self {
        FxState {
            enabled: true,
            last: Instant::now(),
            prev_view: None,
            prev_detail: false,
            prev_flash: 0,
            body: None,
            flash: None,
        }
    }
    /// Effects disabled (e.g. --render text dump, so partial frames don't pollute the output).
    pub fn disabled() -> Self {
        FxState {
            enabled: false,
            ..Self::new()
        }
    }

    /// Frame start: compute elapsed time (dt) + detect state changes to arm effects. dt is passed to render_effect.
    pub fn begin(&mut self, app: &App) -> FxDuration {
        let now = Instant::now();
        // Cap large jumps on the first frame / after a pause (so effects don't teleport).
        let dt: FxDuration = now
            .duration_since(self.last)
            .min(std::time::Duration::from_millis(100))
            .into();
        self.last = now;
        if !self.enabled {
            return dt;
        }
        // Subtle, with a different feel per situation (theme-safe: touches only the fg color → works on light/dark terminals).
        // · view switch = text gently "inks in" from dim to full color (no direction/scatter → easy on the eyes).
        // · detail entry = a brief slight coalesce (zoom-in feel) — a texture distinct from the switch.
        let view_changed = self.prev_view.map(|v| v != app.view).unwrap_or(true);
        let detail_entered = app.detail && !self.prev_detail;
        if detail_entered {
            self.body = Some(fx::coalesce((180u32, Interpolation::QuadOut)));
        } else if view_changed {
            self.body = Some(fx::fade_from_fg(C_DIM(), (150u32, Interpolation::QuadOut)));
        }
        self.prev_view = Some(app.view);
        self.prev_detail = app.detail;
        // New alert → summary bar fg briefly turns red then back to full color (not a full red background → less jarring).
        if app.flash_until > self.prev_flash {
            self.flash = Some(fx::fade_from_fg(C_BAD(), (450u32, Interpolation::QuadOut)));
        }
        self.prev_flash = app.flash_until;
        dt
    }

    /// Apply the body transition (right after widget render, before overlays).
    pub fn body(&mut self, f: &mut Frame, area: Rect, dt: FxDuration) {
        if let Some(e) = self.body.as_mut() {
            f.render_effect(e, area, dt);
            if e.done() {
                self.body = None;
            }
        }
    }

    /// Apply the summary bar flash.
    pub fn flash(&mut self, f: &mut Frame, area: Rect, dt: FxDuration) {
        if let Some(e) = self.flash.as_mut() {
            f.render_effect(e, area, dt);
            if e.done() {
                self.flash = None;
            }
        }
    }

    /// True if an effect is in progress — a hint to spin the render loop faster during animation.
    pub fn animating(&self) -> bool {
        self.body.is_some() || self.flash.is_some()
    }

    /// Toggle animation On/Off (runtime). Turning off immediately clears in-progress effects too. Returns = new state (on).
    pub fn toggle(&mut self) -> bool {
        self.enabled = !self.enabled;
        if !self.enabled {
            self.body = None;
            self.flash = None;
        }
        self.enabled
    }
}
