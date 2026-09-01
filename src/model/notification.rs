use std::collections::VecDeque;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};

const HISTORY_CAPACITY: usize = 500;
const LIVE_CAPACITY: usize = 4;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum NotificationLevel {
    Info,
    Success,
    Warning,
    Error,
}

impl NotificationLevel {
    pub const fn ttl(self) -> Duration {
        match self {
            Self::Success => Duration::from_secs(3),
            Self::Info => Duration::from_secs(4),
            Self::Warning => Duration::from_secs(6),
            Self::Error => Duration::from_secs(8),
        }
    }
}

impl std::fmt::Display for NotificationLevel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Info => "info",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Error => "error",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum NotificationSource {
    Connection,
    Query,
    Catalog,
    Clipboard,
    Profile,
    Editor,
}

impl std::fmt::Display for NotificationSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Connection => "connection",
            Self::Query => "query",
            Self::Catalog => "catalog",
            Self::Clipboard => "clipboard",
            Self::Profile => "profile",
            Self::Editor => "editor",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Notification {
    pub id: u64,
    pub level: NotificationLevel,
    pub title: String,
    pub body: String,
    pub created_at: DateTime<Local>,
    pub source: Option<NotificationSource>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveNotification {
    pub notification_id: u64,
    pub expires_at: Instant,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NotificationCenter {
    history: VecDeque<Notification>,
    live: Vec<LiveNotification>,
    next_id: u64,
}

impl NotificationCenter {
    pub fn push(
        &mut self,
        level: NotificationLevel,
        title: impl Into<String>,
        body: impl Into<String>,
        now: Instant,
    ) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let notification = Notification {
            id,
            level,
            title: title.into(),
            body: body.into(),
            created_at: Local::now(),
            source: None,
        };
        self.history.push_front(notification);
        self.history.truncate(HISTORY_CAPACITY);

        self.live.push(LiveNotification {
            notification_id: id,
            expires_at: now + level.ttl(),
        });
        if self.live.len() > LIVE_CAPACITY {
            self.live.remove(0);
        }
        id
    }

    pub fn push_source(
        &mut self,
        level: NotificationLevel,
        title: impl Into<String>,
        body: impl Into<String>,
        source: NotificationSource,
        now: Instant,
    ) -> u64 {
        let id = self.push(level, title, body, now);
        if let Some(notification) = self.history.front_mut() {
            notification.source = Some(source);
        }
        id
    }

    pub fn history(&self) -> impl DoubleEndedIterator<Item = &Notification> {
        self.history.iter()
    }

    pub fn live(&self) -> &[LiveNotification] {
        &self.live
    }

    pub fn get(&self, id: u64) -> Option<&Notification> {
        self.history
            .iter()
            .find(|notification| notification.id == id)
    }

    pub fn dismiss_live(&mut self, id: u64) -> bool {
        let length = self.live.len();
        self.live
            .retain(|notification| notification.notification_id != id);
        self.live.len() != length
    }

    pub fn dismiss_all_live(&mut self) -> bool {
        let changed = !self.live.is_empty();
        self.live.clear();
        changed
    }

    pub fn expire(&mut self, now: Instant) -> bool {
        let length = self.live.len();
        self.live
            .retain(|notification| notification.expires_at > now);
        self.live.len() != length
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    pub fn clear_all(&mut self) {
        self.history.clear();
        self.live.clear();
    }

    pub fn matching_history<'a>(&'a self, query: &str) -> impl Iterator<Item = &'a Notification> {
        let query = query.to_lowercase();
        self.history.iter().filter(move |notification| {
            query.is_empty() || notification.search_text().contains(&query)
        })
    }
}

impl Notification {
    fn search_text(&self) -> String {
        format!(
            "{} {} {} {} {}",
            self.title,
            self.body,
            self.level,
            self.source
                .map_or(String::new(), |source| source.to_string()),
            self.created_at.format("%H:%M:%S")
        )
        .to_lowercase()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HistorySearchPhase {
    #[default]
    Inactive,
    Editing,
    Confirmed,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NotificationHistoryState {
    pub query: String,
    pub phase: HistorySearchPhase,
    pub selected: usize,
    pub active_match: usize,
    previous_query: String,
    pub clear_confirm: bool,
}

impl NotificationHistoryState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn begin_search(&mut self) {
        self.previous_query.clone_from(&self.query);
        self.phase = HistorySearchPhase::Editing;
    }

    pub fn request_clear(&mut self) {
        self.clear_confirm = true;
    }

    pub fn cancel_clear(&mut self) {
        self.clear_confirm = false;
    }

    pub fn push_search_char(&mut self, character: char) {
        if self.phase == HistorySearchPhase::Editing {
            self.query.push(character);
        }
    }

    pub fn backspace_search(&mut self) {
        if self.phase == HistorySearchPhase::Editing {
            self.query.pop();
        }
    }

    pub fn clear_search(&mut self) {
        if self.phase == HistorySearchPhase::Editing {
            self.query.clear();
        }
    }

    pub fn confirm_search(&mut self, history: &[Notification]) {
        self.phase = HistorySearchPhase::Confirmed;
        self.active_match = 0;
        let selected = self.matching_indices(history).next().unwrap_or(0);
        self.selected = selected;
    }

    pub fn cancel_search(&mut self) {
        if self.phase == HistorySearchPhase::Editing {
            self.query.clone_from(&self.previous_query);
            self.phase = if self.query.is_empty() {
                HistorySearchPhase::Inactive
            } else {
                HistorySearchPhase::Confirmed
            };
        }
    }

    pub fn move_selection(&mut self, delta: isize, history_len: usize) {
        self.selected = move_bounded(self.selected, delta, history_len);
    }

    pub fn next_match(&mut self, history: &[Notification]) {
        self.move_match(history, 1);
    }

    pub fn previous_match(&mut self, history: &[Notification]) {
        self.move_match(history, -1);
    }

    pub fn matching_indices<'a>(
        &'a self,
        history: &'a [Notification],
    ) -> impl Iterator<Item = usize> + 'a {
        let query = self.query.to_lowercase();
        history
            .iter()
            .enumerate()
            .filter(move |(_, notification)| {
                query.is_empty() || notification.search_text().contains(&query)
            })
            .map(|(index, _)| index)
    }

    fn move_match(&mut self, history: &[Notification], delta: isize) {
        let matches: Vec<_> = self.matching_indices(history).collect();
        if matches.is_empty() {
            self.selected = 0;
            self.active_match = 0;
            return;
        }
        self.active_match = move_wrapped(self.active_match, delta, matches.len());
        self.selected = matches[self.active_match];
    }
}

fn move_wrapped(current: usize, delta: isize, length: usize) -> usize {
    if length == 0 {
        return 0;
    }
    (current as isize + delta).rem_euclid(length as isize) as usize
}

fn move_bounded(current: usize, delta: isize, length: usize) -> usize {
    if length == 0 {
        return 0;
    }
    current
        .saturating_add_signed(delta)
        .min(length.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn center() -> NotificationCenter {
        NotificationCenter::default()
    }

    #[test]
    fn push_uses_level_ttl_and_expiry_keeps_history() {
        let now = Instant::now();
        let mut center = center();
        let id = center.push(NotificationLevel::Warning, "Title", "Body", now);
        assert_eq!(center.live()[0].expires_at, now + Duration::from_secs(6));
        assert!(!center.expire(now + Duration::from_secs(5)));
        assert!(center.expire(now + Duration::from_secs(6)));
        assert_eq!(center.history().count(), 1);
        assert_eq!(center.history().next().unwrap().id, id);
    }

    #[test]
    fn live_dismissal_is_independent_and_capacity_is_bounded() {
        let now = Instant::now();
        let mut center = center();
        for index in 0..6 {
            center.push(NotificationLevel::Info, index.to_string(), "", now);
        }
        assert_eq!(center.history().count(), 6);
        assert_eq!(center.live().len(), 4);
        assert!(center.dismiss_live(5));
        assert_eq!(center.live().len(), 3);
        assert!(center.dismiss_all_live());
        assert!(center.live().is_empty());
        assert!(!center.dismiss_all_live());
    }

    #[test]
    fn history_is_capped_and_can_be_cleared() {
        let mut center = center();
        let now = Instant::now();
        for index in 0..501 {
            center.push(NotificationLevel::Info, index.to_string(), "", now);
        }
        assert_eq!(center.history().count(), 500);
        assert_eq!(center.history().next().unwrap().title, "500");
        center.clear_history();
        assert_eq!(center.history().count(), 0);
        assert_eq!(center.live().len(), 4);
    }

    #[test]
    fn clear_all_removes_history_and_live_notifications() {
        let now = Instant::now();
        let mut center = center();
        center.push(NotificationLevel::Info, "Title", "Body", now);
        center.clear_all();
        assert_eq!(center.history().count(), 0);
        assert!(center.live().is_empty());
    }

    #[test]
    fn search_matches_case_insensitively_and_navigation_wraps() {
        let now = Instant::now();
        let mut center = center();
        center.push(NotificationLevel::Error, "Connection", "Refused", now);
        center.push(NotificationLevel::Info, "Other", "connection retry", now);
        center.push(NotificationLevel::Success, "Done", "All good", now);
        let history: Vec<_> = center.history().cloned().collect();
        let mut state = NotificationHistoryState::new();
        state.begin_search();
        for character in "CONNECTION".chars() {
            state.push_search_char(character);
        }
        state.confirm_search(&history);
        assert_eq!(state.selected, 1);
        state.next_match(&history);
        assert_eq!(state.selected, 2);
        state.next_match(&history);
        assert_eq!(state.selected, 1);
        state.previous_match(&history);
        assert_eq!(state.selected, 2);
        state.move_selection(1, history.len());
        assert_eq!(state.selected, 2);
        state.move_selection(-10, history.len());
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn every_level_has_the_declared_ttl() {
        assert_eq!(NotificationLevel::Success.ttl(), Duration::from_secs(3));
        assert_eq!(NotificationLevel::Info.ttl(), Duration::from_secs(4));
        assert_eq!(NotificationLevel::Warning.ttl(), Duration::from_secs(6));
        assert_eq!(NotificationLevel::Error.ttl(), Duration::from_secs(8));
    }
}
