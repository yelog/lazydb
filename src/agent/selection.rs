use std::fmt;

use uuid::Uuid;

use super::context::{AgentProfileScope, VisibleAgentProfile};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentErrorCode {
    ConnectionNotFound,
    ConnectionAmbiguous,
    NoVisibleConnections,
}

impl AgentErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConnectionNotFound => "connection_not_found",
            Self::ConnectionAmbiguous => "connection_ambiguous",
            Self::NoVisibleConnections => "no_visible_connections",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentError {
    pub code: AgentErrorCode,
    pub message: String,
}

impl fmt::Display for AgentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AgentError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionReason {
    ExplicitUuid,
    ExplicitName,
    SoleProject,
    SoleGlobal,
}

#[derive(Debug)]
pub struct SelectedAgentProfile<'a> {
    pub profile: &'a VisibleAgentProfile<'a>,
    pub reason: SelectionReason,
}

pub fn select_profile<'a>(
    visible: &'a [VisibleAgentProfile<'a>],
    selector: Option<&str>,
) -> Result<SelectedAgentProfile<'a>, AgentError> {
    if visible.is_empty() {
        return Err(error(
            AgentErrorCode::NoVisibleConnections,
            "no connections are available for this project",
        ));
    }

    if let Some(selector) = selector.map(str::trim).filter(|value| !value.is_empty()) {
        if let Ok(id) = Uuid::parse_str(selector) {
            return visible
                .iter()
                .find(|entry| entry.profile.id == id)
                .map(|profile| SelectedAgentProfile {
                    profile,
                    reason: SelectionReason::ExplicitUuid,
                })
                .ok_or_else(|| error(AgentErrorCode::ConnectionNotFound, "connection not found"));
        }

        let matches = visible
            .iter()
            .filter(|entry| entry.profile.name == selector)
            .collect::<Vec<_>>();
        return match matches.as_slice() {
            [profile] => Ok(SelectedAgentProfile {
                profile,
                reason: SelectionReason::ExplicitName,
            }),
            [] => Err(error(
                AgentErrorCode::ConnectionNotFound,
                "connection not found",
            )),
            _ => Err(error(
                AgentErrorCode::ConnectionAmbiguous,
                "connection name is ambiguous",
            )),
        };
    }

    let project = visible
        .iter()
        .filter(|entry| entry.scope == AgentProfileScope::CurrentProject)
        .collect::<Vec<_>>();
    if project.len() == 1 {
        return Ok(SelectedAgentProfile {
            profile: project[0],
            reason: SelectionReason::SoleProject,
        });
    }
    if project.len() > 1 {
        return Err(error(
            AgentErrorCode::ConnectionAmbiguous,
            "multiple current-project connections require an explicit selector",
        ));
    }

    let global = visible
        .iter()
        .filter(|entry| entry.scope == AgentProfileScope::Global)
        .collect::<Vec<_>>();
    if global.len() == 1 {
        return Ok(SelectedAgentProfile {
            profile: global[0],
            reason: SelectionReason::SoleGlobal,
        });
    }
    Err(error(
        AgentErrorCode::ConnectionAmbiguous,
        "multiple global connections require an explicit selector",
    ))
}

fn error(code: AgentErrorCode, message: &str) -> AgentError {
    AgentError {
        code,
        message: message.to_owned(),
    }
}
