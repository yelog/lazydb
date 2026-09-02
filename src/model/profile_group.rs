use uuid::Uuid;

use super::text_input::TextInput;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProfileGroupOverlay {
    Picker {
        profile_id: Uuid,
        selected: usize,
        busy: bool,
    },
    Edit {
        group_id: Option<Uuid>,
        name: TextInput,
        error: Option<String>,
        busy: bool,
    },
    DeleteConfirm {
        group_id: Uuid,
        member_count: usize,
        busy: bool,
    },
}
