use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextDetailRequest {
    pub title: String,
    pub source_session_id: Uuid,
    pub source_revision: u64,
    pub display_text: String,
    pub copy_text: String,
    pub return_overlay: Option<Box<super::workspace::Overlay>>,
}

impl TextDetailRequest {
    pub fn new(
        title: impl Into<String>,
        source_session_id: Uuid,
        source_revision: u64,
        display_text: impl Into<String>,
        copy_text: impl Into<String>,
        return_overlay: Option<Box<super::workspace::Overlay>>,
    ) -> Self {
        Self {
            title: title.into(),
            source_session_id,
            source_revision,
            display_text: display_text.into(),
            copy_text: copy_text.into(),
            return_overlay,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextDetailState {
    pub title: String,
    pub session_id: Uuid,
    pub source_session_id: Uuid,
    pub source_revision: u64,
    pub revision: u64,
    pub copy_text: String,
    pub return_overlay: Option<Box<super::workspace::Overlay>>,
}

#[cfg(test)]
mod tests {
    use super::TextDetailRequest;
    use uuid::Uuid;

    #[test]
    fn request_keeps_display_and_complete_copy_text_separate() {
        let request = TextDetailRequest::new("x", Uuid::nil(), 4, "safe", "full", None);
        assert_eq!(request.display_text, "safe");
        assert_eq!(request.copy_text, "full");
    }
}
