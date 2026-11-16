use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use uuid::Uuid;

use crate::{
    agent::policy::WritePolicy,
    commands::CommandContext,
    data_ops::{
        DataEndpoint, DataJobPlan, DataJobRecord, DataJobSpec, DataOperation, NativeToolPaths,
    },
};

use super::text_input::{TextInput, TextInputEdit};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataField {
    SourceProfile,
    SourceDatabase,
    TargetProfile,
    TargetDatabase,
    Artifact,
    DumpTool,
    RestoreTool,
    Review,
    Cancel,
}

impl DataField {
    pub const fn label(self) -> &'static str {
        match self {
            Self::SourceProfile => "Source profile",
            Self::SourceDatabase => "Source database / file",
            Self::TargetProfile => "Target profile",
            Self::TargetDatabase => "NEW database / file",
            Self::Artifact => "Artifact path",
            Self::DumpTool => "Dump tool (optional)",
            Self::RestoreTool => "Restore tool (optional)",
            Self::Review => "Review Plan",
            Self::Cancel => "Cancel",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataOperationAction {
    Open {
        operation: DataOperation,
        context: CommandContext,
    },
    Edit(TextInputEdit),
    Paste(String),
    FocusNext(bool),
    FocusField(DataField),
    CycleProfile(isize),
    Review,
    Execute,
    Back,
    Close,
    Scroll(isize),
    PlanReady {
        request_id: Uuid,
        result: Result<Box<DataJobPlan>, String>,
    },
    OpenTasks,
    RefreshTasks,
    TasksLoaded {
        request_id: Uuid,
        result: Result<Vec<DataJobRecord>, String>,
    },
    TaskMove(isize),
    TaskSelect(usize),
    CancelTask,
    RerunTask,
    JobUpdated {
        request_id: Uuid,
        record: Box<DataJobRecord>,
    },
    JobFailed {
        request_id: Uuid,
        job_id: Uuid,
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct DataOperationDraft {
    pub operation: DataOperation,
    pub source_profile_id: Option<Uuid>,
    pub target_profile_id: Option<Uuid>,
    pub source_database: TextInput,
    pub target_database: TextInput,
    pub artifact: TextInput,
    pub dump_tool: TextInput,
    pub restore_tool: TextInput,
    pub focus: DataField,
    pub pending_plan: Option<Uuid>,
    pub plan: Option<DataJobPlan>,
    pub error: Option<String>,
    pub scroll: usize,
}

impl DataOperationDraft {
    pub fn new(operation: DataOperation, endpoint: Option<DataEndpoint>) -> Self {
        let mut draft = Self {
            operation,
            source_profile_id: endpoint.as_ref().map(|endpoint| endpoint.profile_id),
            target_profile_id: endpoint.as_ref().map(|endpoint| endpoint.profile_id),
            source_database: TextInput::default(),
            target_database: TextInput::default(),
            artifact: TextInput::default(),
            dump_tool: TextInput::default(),
            restore_tool: TextInput::default(),
            focus: if operation == DataOperation::Restore {
                DataField::Artifact
            } else {
                DataField::SourceDatabase
            },
            pending_plan: None,
            plan: None,
            error: None,
            scroll: 0,
        };
        if let Some(endpoint) = endpoint {
            draft.source_database.set(endpoint.database);
        }
        draft
    }

    pub fn from_spec(spec: &DataJobSpec) -> Self {
        let mut draft = Self::new(spec.operation, spec.source.clone());
        draft.target_profile_id = spec
            .destination
            .as_ref()
            .map(|endpoint| endpoint.profile_id);
        if let Some(destination) = &spec.destination {
            draft.target_database.set(&destination.database);
        }
        draft.artifact.set(spec.artifact.to_string_lossy());
        if let Some(path) = &spec.tools.dump {
            draft.dump_tool.set(path.to_string_lossy());
        }
        if let Some(path) = &spec.tools.restore {
            draft.restore_tool.set(path.to_string_lossy());
        }
        draft
    }

    pub fn fields(&self) -> Vec<DataField> {
        let mut fields = Vec::new();
        if self.operation != DataOperation::Restore {
            fields.extend([DataField::SourceProfile, DataField::SourceDatabase]);
        }
        if self.operation.writes_database() {
            fields.extend([DataField::TargetProfile, DataField::TargetDatabase]);
        }
        fields.push(DataField::Artifact);
        if self.operation != DataOperation::Restore {
            fields.push(DataField::DumpTool);
        }
        if self.operation.writes_database() {
            fields.push(DataField::RestoreTool);
        }
        fields.extend([DataField::Review, DataField::Cancel]);
        fields
    }

    pub fn input(&self, field: DataField) -> Option<&TextInput> {
        match field {
            DataField::SourceDatabase => Some(&self.source_database),
            DataField::TargetDatabase => Some(&self.target_database),
            DataField::Artifact => Some(&self.artifact),
            DataField::DumpTool => Some(&self.dump_tool),
            DataField::RestoreTool => Some(&self.restore_tool),
            _ => None,
        }
    }

    pub fn input_mut(&mut self) -> Option<&mut TextInput> {
        match self.focus {
            DataField::SourceDatabase => Some(&mut self.source_database),
            DataField::TargetDatabase => Some(&mut self.target_database),
            DataField::Artifact => Some(&mut self.artifact),
            DataField::DumpTool => Some(&mut self.dump_tool),
            DataField::RestoreTool => Some(&mut self.restore_tool),
            _ => None,
        }
    }

    pub fn spec(&self) -> Result<DataJobSpec, String> {
        let source = if self.operation != DataOperation::Restore {
            Some(DataEndpoint {
                profile_id: self.source_profile_id.ok_or("Select a source profile")?,
                database: required(&self.source_database, "Enter the source database / file")?,
            })
        } else {
            None
        };
        let destination = if self.operation.writes_database() {
            Some(DataEndpoint {
                profile_id: self.target_profile_id.ok_or("Select a target profile")?,
                database: required(&self.target_database, "Enter a NEW target database / file")?,
            })
        } else {
            None
        };
        Ok(DataJobSpec {
            operation: self.operation,
            source,
            destination,
            artifact: PathBuf::from(required(&self.artifact, "Enter the artifact path")?),
            tools: NativeToolPaths {
                dump: optional_path(&self.dump_tool),
                restore: optional_path(&self.restore_tool),
            },
        })
    }
}

fn required(input: &TextInput, message: &str) -> Result<String, String> {
    let value = input.value().trim();
    if value.is_empty() {
        return Err(message.to_owned());
    }
    if value.chars().any(char::is_control) {
        return Err("Fields must not contain control characters".to_owned());
    }
    Ok(value.to_owned())
}

fn optional_path(input: &TextInput) -> Option<PathBuf> {
    let value = input.value().trim();
    (!value.is_empty()).then(|| PathBuf::from(value))
}

pub struct DataOperationsState {
    pub draft: Option<DataOperationDraft>,
    pub records: Vec<DataJobRecord>,
    pub selected: usize,
    pub scroll: usize,
    pub history_request: Option<Uuid>,
    pub history_error: Option<String>,
    pub runs: HashMap<Uuid, Uuid>,
    pub cancelling: HashSet<Uuid>,
    pub write_policy: WritePolicy,
}

impl Default for DataOperationsState {
    fn default() -> Self {
        Self {
            draft: None,
            records: Vec::new(),
            selected: 0,
            scroll: 0,
            history_request: None,
            history_error: None,
            runs: HashMap::new(),
            cancelling: HashSet::new(),
            write_policy: WritePolicy::All,
        }
    }
}

impl DataOperationsState {
    pub fn selected_record(&self) -> Option<&DataJobRecord> {
        self.records.get(self.selected)
    }

    pub fn merge_record(&mut self, record: DataJobRecord) {
        let selected = self.selected_record().map(|record| record.id);
        if let Some(existing) = self.records.iter_mut().find(|entry| entry.id == record.id) {
            if existing.updated_at > record.updated_at
                || (existing.status.is_terminal() && !record.status.is_terminal())
            {
                return;
            }
            *existing = record;
        } else {
            self.records.push(record);
        }
        self.records
            .sort_by_key(|entry| std::cmp::Reverse(entry.updated_at));
        if let Some(selected) = selected {
            self.selected = self
                .records
                .iter()
                .position(|record| record.id == selected)
                .unwrap_or_default();
        }
    }
}
