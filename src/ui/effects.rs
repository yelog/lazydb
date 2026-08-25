use std::time::{Duration, Instant};

use ratatui::{Frame, layout::Rect, style::Color};
use tachyonfx::{Effect, EffectRenderer, Interpolation, fx};

#[derive(Debug)]
pub struct UiEffects {
    active: Option<Effect>,
    last_tick: Instant,
    reduced_motion: bool,
}

impl UiEffects {
    pub fn new(reduced_motion: bool) -> Self {
        Self {
            active: None,
            last_tick: Instant::now(),
            reduced_motion,
        }
    }

    pub fn focus_changed(&mut self, color: Color) {
        if self.reduced_motion {
            return;
        }
        self.active = Some(fx::fade_from_fg(color, (120, Interpolation::SineOut)));
        self.last_tick = Instant::now();
    }

    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let Some(effect) = self.active.as_mut() else {
            return;
        };
        let now = Instant::now();
        let elapsed = now
            .saturating_duration_since(self.last_tick)
            .min(Duration::from_millis(34));
        self.last_tick = now;
        frame.render_effect(effect, area, elapsed);
        if effect.done() {
            self.active = None;
        }
    }

    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }
}
