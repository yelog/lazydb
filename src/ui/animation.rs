use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use ratatui::{Frame, layout::Rect, style::Color};
use tachyonfx::{Effect, EffectRenderer, Interpolation, fx};
use uuid::Uuid;

use crate::{cli::MotionMode, model::relation::RelationRequest};

pub(crate) const LOADING_DELAY: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum LoadIdentity {
    Query { tab_id: Uuid, generation: u64 },
    Derived { tab_id: Uuid, generation: u64 },
    Relation(RelationRequest),
    ProfileScope { request_id: u64 },
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ResultIdentity {
    Query { tab_id: Uuid, generation: u64 },
    Derived { tab_id: Uuid, generation: u64 },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum EffectKind {
    Overlay,
    Result,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AnimationObservation {
    pub active_loads: HashSet<LoadIdentity>,
    pub result: Option<ResultIdentity>,
}

pub(crate) fn spinner_frame(mode: MotionMode, elapsed: Duration, frames: usize) -> usize {
    if frames == 0 || mode == MotionMode::Off {
        return 0;
    }
    let frame_ms = match mode {
        MotionMode::Full => 100,
        MotionMode::Reduced => 200,
        MotionMode::Off => unreachable!(),
    };
    (elapsed.as_millis() / frame_ms % frames as u128) as usize
}

pub(crate) fn show_loading_helper(elapsed: Duration) -> bool {
    elapsed >= LOADING_DELAY
}

#[derive(Debug)]
pub(crate) struct AnimationState {
    mode: MotionMode,
    now: Instant,
    active_loads: HashMap<LoadIdentity, Instant>,
    observed_loads: HashSet<LoadIdentity>,
    observed_result: Option<ResultIdentity>,
    result_ready: Option<ResultIdentity>,
    last_visible_frame: Option<u128>,
    effect: Option<Effect>,
    effect_area: Option<Rect>,
    effect_kind: Option<EffectKind>,
    last_effect_at: Instant,
    overlay_key: Option<u8>,
}

impl AnimationState {
    pub(crate) fn new(mode: MotionMode, now: Instant) -> Self {
        Self {
            mode,
            now,
            active_loads: HashMap::new(),
            observed_loads: HashSet::new(),
            observed_result: None,
            result_ready: None,
            last_visible_frame: None,
            effect: None,
            effect_area: None,
            effect_kind: None,
            last_effect_at: now,
            overlay_key: None,
        }
    }

    pub(crate) fn mode(&self) -> MotionMode {
        self.mode
    }

    pub(crate) fn set_now(&mut self, now: Instant) {
        self.now = now;
    }

    pub(crate) fn track_load(&mut self, identity: LoadIdentity) {
        self.active_loads.entry(identity).or_insert(self.now);
    }

    #[cfg(test)]
    pub(crate) fn finish_load(&mut self, identity: &LoadIdentity) {
        self.active_loads.remove(identity);
    }

    pub(crate) fn elapsed(&self, identity: &LoadIdentity) -> Option<Duration> {
        self.active_loads
            .get(identity)
            .map(|started| self.now.saturating_duration_since(*started))
    }

    #[cfg(test)]
    pub(crate) fn has_active_loads(&self) -> bool {
        !self.active_loads.is_empty()
    }

    pub(crate) fn advance(&mut self, now: Instant) -> bool {
        let previous = self.visible_frame();
        self.now = now;
        let current = self.visible_frame();
        let changed = previous != current;
        self.last_visible_frame = Some(current);
        changed || self.has_active_effects()
    }

    fn visible_frame(&self) -> u128 {
        if self.active_loads.is_empty() {
            return 0;
        }
        let elapsed = self
            .active_loads
            .values()
            .map(|started| self.now.saturating_duration_since(*started))
            .max()
            .unwrap_or_default();
        match self.mode {
            MotionMode::Full => elapsed.as_millis() / 100,
            MotionMode::Reduced => elapsed.as_millis() / 200,
            MotionMode::Off => self
                .active_loads
                .values()
                .map(|started| self.now.saturating_duration_since(*started))
                .any(|elapsed| elapsed >= LOADING_DELAY) as u128,
        }
    }

    pub(crate) fn observe(&mut self, observation: AnimationObservation) {
        for identity in &observation.active_loads {
            if !self.observed_loads.contains(identity) {
                self.track_load(identity.clone());
            }
        }
        self.active_loads
            .retain(|identity, _| observation.active_loads.contains(identity));
        self.observed_loads = observation.active_loads;

        if observation.result != self.observed_result {
            if let Some(result) = observation.result.clone() {
                self.result_ready = Some(result);
            }
            self.observed_result = observation.result;
        }
    }

    pub(crate) fn take_result_ready(&mut self) -> Option<ResultIdentity> {
        self.result_ready.take()
    }

    pub(crate) fn start_effect(&mut self, kind: EffectKind, area: Rect) {
        if self.mode != MotionMode::Full || area.width == 0 || area.height == 0 {
            return;
        }
        self.effect =
            Some(fx::fade_from_fg(Color::Black, (160, Interpolation::QuadOut)).with_area(area));
        self.effect_area = Some(area);
        self.effect_kind = Some(kind);
        self.last_effect_at = self.now;
    }

    pub(crate) fn cancel_effect(&mut self) {
        self.effect = None;
        self.effect_area = None;
        self.effect_kind = None;
    }

    pub(crate) fn has_active_effects(&self) -> bool {
        self.effect.is_some()
    }

    pub(crate) fn render_effect(&mut self, frame: &mut Frame<'_>, now: Instant) {
        let Some(effect) = self.effect.as_mut() else {
            return;
        };
        let Some(area) = self.effect_area else {
            self.cancel_effect();
            return;
        };
        let elapsed = now.saturating_duration_since(self.last_effect_at);
        frame.render_effect(effect, area, elapsed.into());
        self.last_effect_at = now;
        if effect.done() {
            self.cancel_effect();
        }
    }

    pub(crate) fn prepare_overlay(&mut self, key: u8, area: Rect) {
        if self.overlay_key != Some(key) {
            self.overlay_key = Some(key);
            self.start_effect(EffectKind::Overlay, area);
        }
    }

    pub(crate) fn clear_overlay(&mut self) {
        self.overlay_key = None;
        if self.effect_kind == Some(EffectKind::Overlay) {
            self.cancel_effect();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use uuid::Uuid;

    use super::{
        AnimationObservation, AnimationState, LOADING_DELAY, LoadIdentity, ResultIdentity,
        show_loading_helper, spinner_frame,
    };
    use crate::cli::MotionMode;

    #[test]
    fn full_motion_advances_spinner_every_hundred_milliseconds() {
        assert_eq!(spinner_frame(MotionMode::Full, Duration::ZERO, 10), 0);
        assert_eq!(
            spinner_frame(MotionMode::Full, Duration::from_millis(99), 10),
            0
        );
        assert_eq!(
            spinner_frame(MotionMode::Full, Duration::from_millis(100), 10),
            1
        );
    }

    #[test]
    fn reduced_motion_uses_a_lower_cadence_and_off_is_stable() {
        assert_eq!(
            spinner_frame(MotionMode::Reduced, Duration::from_millis(199), 10),
            0
        );
        assert_eq!(
            spinner_frame(MotionMode::Reduced, Duration::from_millis(200), 10),
            1
        );
        assert_eq!(
            spinner_frame(MotionMode::Off, Duration::from_secs(10), 10),
            0
        );
    }

    #[test]
    fn loading_helper_is_hidden_before_the_delay() {
        assert!(!show_loading_helper(Duration::from_millis(249)));
        assert!(show_loading_helper(LOADING_DELAY));
    }

    #[test]
    fn load_tracking_preserves_start_time_until_finished() {
        let start = Instant::now();
        let identity = LoadIdentity::Query {
            tab_id: Uuid::nil(),
            generation: 1,
        };
        let mut state = AnimationState::new(MotionMode::Full, start);
        state.track_load(identity.clone());
        state.set_now(start + Duration::from_millis(500));
        state.track_load(identity.clone());

        assert_eq!(state.elapsed(&identity), Some(Duration::from_millis(500)));
        assert!(state.has_active_loads());

        state.finish_load(&identity);
        assert_eq!(state.elapsed(&identity), None);
        assert!(!state.has_active_loads());
    }

    #[test]
    fn observing_the_same_load_preserves_its_start_time() {
        let start = Instant::now();
        let identity = LoadIdentity::Query {
            tab_id: Uuid::nil(),
            generation: 1,
        };
        let mut state = AnimationState::new(MotionMode::Full, start);
        state.observe(AnimationObservation {
            active_loads: [identity.clone()].into_iter().collect(),
            result: None,
        });
        state.set_now(start + Duration::from_millis(500));
        state.observe(AnimationObservation {
            active_loads: [identity.clone()].into_iter().collect(),
            result: None,
        });
        assert_eq!(state.elapsed(&identity), Some(Duration::from_millis(500)));
    }

    #[test]
    fn observing_a_new_result_queues_one_ready_transition() {
        let now = Instant::now();
        let result = ResultIdentity::Query {
            tab_id: Uuid::nil(),
            generation: 1,
        };
        let mut state = AnimationState::new(MotionMode::Full, now);
        let observation = AnimationObservation {
            active_loads: Default::default(),
            result: Some(result.clone()),
        };
        state.observe(observation.clone());
        assert_eq!(state.take_result_ready(), Some(result));
        state.observe(observation);
        assert_eq!(state.take_result_ready(), None);
    }

    #[test]
    fn animation_redraws_only_when_the_visible_frame_changes() {
        let start = Instant::now();
        let identity = LoadIdentity::Query {
            tab_id: Uuid::nil(),
            generation: 1,
        };
        let mut state = AnimationState::new(MotionMode::Full, start);
        state.observe(AnimationObservation {
            active_loads: [identity].into_iter().collect(),
            result: None,
        });

        assert!(!state.advance(start));
        assert!(!state.advance(start + Duration::from_millis(99)));
        assert!(state.advance(start + Duration::from_millis(100)));
        assert!(!state.advance(start + Duration::from_millis(150)));
    }

    #[test]
    fn off_motion_redraws_at_the_loading_threshold_only() {
        let start = Instant::now();
        let identity = LoadIdentity::Query {
            tab_id: Uuid::nil(),
            generation: 1,
        };
        let mut state = AnimationState::new(MotionMode::Off, start);
        state.observe(AnimationObservation {
            active_loads: [identity].into_iter().collect(),
            result: None,
        });

        assert!(!state.advance(start));
        assert!(!state.advance(start + Duration::from_millis(149)));
        assert!(state.advance(start + LOADING_DELAY));
        assert!(!state.advance(start + Duration::from_millis(500)));
    }
}
