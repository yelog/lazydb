use std::path::{Path, PathBuf};

use crate::{
    profile::{ConnectionProfile, ProfileAccess},
    project::{ProjectContext, ProjectContextError},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentProjectContext {
    pub project: ProjectContext,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentProfileScope {
    CurrentProject,
    Global,
}

#[derive(Clone, Copy, Debug)]
pub struct VisibleAgentProfile<'a> {
    pub profile: &'a ConnectionProfile,
    pub scope: AgentProfileScope,
}

impl AgentProjectContext {
    pub fn resolve(project: Option<&Path>) -> Result<Self, ProjectContextError> {
        let project = match project {
            Some(path) => ProjectContext::resolve_from(path)?,
            None => ProjectContext::resolve_current()?,
        };
        Ok(Self { project })
    }

    pub fn root(&self) -> &Path {
        &self.project.root
    }

    pub fn visible_profiles<'a>(
        &self,
        profiles: &'a [ConnectionProfile],
    ) -> Vec<VisibleAgentProfile<'a>> {
        profiles
            .iter()
            .filter_map(|profile| {
                let scope = match &profile.access {
                    ProfileAccess::Global => AgentProfileScope::Global,
                    ProfileAccess::Projects { roots }
                        if roots.iter().any(|root| root == self.root()) =>
                    {
                        AgentProfileScope::CurrentProject
                    }
                    ProfileAccess::Projects { .. } => return None,
                };
                Some(VisibleAgentProfile { profile, scope })
            })
            .collect()
    }
}

pub fn canonical_project_root(path: &Path) -> Result<PathBuf, ProjectContextError> {
    Ok(ProjectContext::resolve_from(path)?.root)
}
