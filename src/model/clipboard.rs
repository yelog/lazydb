use std::time::{Duration, Instant};

const NOTICE_TTL: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClipboardNoticeKind {
    Success,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipboardNotice {
    pub message: String,
    pub kind: ClipboardNoticeKind,
    expires_at: Instant,
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{ClipboardNotice, ClipboardNoticeKind};

    #[test]
    fn notices_expire_after_two_seconds() {
        let now = Instant::now();
        let notice = ClipboardNotice::success("Copied cell", now);
        assert_eq!(notice.kind, ClipboardNoticeKind::Success);
        assert!(!notice.is_expired(now + Duration::from_secs(1)));
        assert!(notice.is_expired(now + Duration::from_secs(2)));
    }
}

impl ClipboardNotice {
    pub fn success(message: impl Into<String>, now: Instant) -> Self {
        Self::new(message, ClipboardNoticeKind::Success, now)
    }

    pub fn error(message: impl Into<String>, now: Instant) -> Self {
        Self::new(message, ClipboardNoticeKind::Error, now)
    }

    pub fn is_expired(&self, now: Instant) -> bool {
        now >= self.expires_at
    }

    fn new(message: impl Into<String>, kind: ClipboardNoticeKind, now: Instant) -> Self {
        Self {
            message: message.into(),
            kind,
            expires_at: now + NOTICE_TTL,
        }
    }
}
