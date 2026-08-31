use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectContext {
    pub root: PathBuf,
    pub display_name: String,
}

#[derive(Debug, Error)]
pub enum ProjectContextError {
    #[error("failed to canonicalize project start `{path}`: {source}")]
    Canonicalize {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("project root `{path}` has no display name")]
    MissingDisplayName { path: PathBuf },
}

impl ProjectContext {
    pub fn resolve_current() -> Result<Self, ProjectContextError> {
        let current =
            std::env::current_dir().map_err(|source| ProjectContextError::Canonicalize {
                path: PathBuf::from("."),
                source,
            })?;
        Self::resolve_from(&current)
    }

    pub fn resolve_from(start: &Path) -> Result<Self, ProjectContextError> {
        let canonical_start =
            start
                .canonicalize()
                .map_err(|source| ProjectContextError::Canonicalize {
                    path: start.to_owned(),
                    source,
                })?;
        let mut candidate = canonical_start.as_path();
        let root = loop {
            if candidate.join(".git").exists() {
                break candidate.to_owned();
            }
            let Some(parent) = candidate.parent() else {
                break canonical_start.clone();
            };
            if parent == candidate {
                break canonical_start.clone();
            }
            candidate = parent;
        };
        let display_name = root
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| ProjectContextError::MissingDisplayName { path: root.clone() })?
            .to_owned();
        Ok(Self { root, display_name })
    }
}
