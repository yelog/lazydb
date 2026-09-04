use crate::update::{UpdateInspection, UpdateStatus};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdateOperation {
    Check,
    Install,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum UpdateState {
    #[default]
    Idle,
    Checking {
        request_id: u64,
        automatic: bool,
    },
    UpToDate(UpdateInspection),
    Available(UpdateInspection),
    Installing {
        request_id: u64,
        inspection: UpdateInspection,
    },
    ReadyToRestart(UpdateInspection),
    ManagerActionRequired(UpdateInspection),
    Failed {
        operation: UpdateOperation,
        message: String,
    },
}

impl UpdateState {
    pub fn from_inspection(inspection: UpdateInspection) -> Self {
        match inspection.status {
            UpdateStatus::UpToDate => Self::UpToDate(inspection),
            UpdateStatus::Available => Self::Available(inspection),
            UpdateStatus::ReadyToRestart => Self::ReadyToRestart(inspection),
            UpdateStatus::ManagerActionRequired => Self::ManagerActionRequired(inspection),
            UpdateStatus::Error => Self::Failed {
                operation: UpdateOperation::Check,
                message: inspection
                    .action
                    .clone()
                    .unwrap_or_else(|| "update check failed".into()),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdateOverlayFocus {
    Later,
    Primary,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateOverlayState {
    pub focus: UpdateOverlayFocus,
}
