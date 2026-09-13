//! OpenCode MCP configuration compatibility helpers.
//!
//! OpenCode V1 stores servers below `mcp`, while native V2 stores them below
//! `mcp.servers`. Keep the two representations explicit so setup and doctor
//! can share the same resolution rules without rewriting user configuration.

use anyhow::{Result, bail};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub enum Format {
    V1,
    V2,
}

impl Format {
    pub(crate) fn path(self) -> &'static [&'static str] {
        match self {
            Self::V1 => &["mcp", "lazydb"],
            Self::V2 => &["mcp", "servers", "lazydb"],
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::V2 => "v2",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    pub format: Format,
}

#[derive(Clone, Debug)]
pub(crate) struct Resolved {
    pub effective: Candidate,
    pub(crate) shadowed: Vec<Candidate>,
}

pub(crate) fn resolve(root: &Value) -> Result<Option<Resolved>> {
    if !root.is_object() {
        bail!("configuration must be an object");
    }

    let mut candidates = Vec::new();
    // V2 is deliberately inspected first: when both valid forms exist in the
    // same document, the native V2 value is the canonical one.
    for format in [Format::V2, Format::V1] {
        if let Some(value) = value_at(root, format.path())? {
            if !value.is_object() {
                bail!("OpenCode {} LazyDB entry must be an object", format.name());
            }
            candidates.push(Candidate { format });
        }
    }

    let Some(effective) = candidates.first().cloned() else {
        return Ok(None);
    };
    Ok(Some(Resolved {
        effective,
        shadowed: candidates.into_iter().skip(1).collect(),
    }))
}

fn value_at<'a>(root: &'a Value, path: &[&str]) -> Result<Option<&'a Value>> {
    let mut current = root;
    for key in path {
        let Some(object) = current.as_object() else {
            bail!("OpenCode configuration parent for `{key}` must be an object");
        };
        let Some(value) = object.get(*key) else {
            return Ok(None);
        };
        current = value;
    }
    Ok(Some(current))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_v1_server() {
        let root = serde_json::json!({
            "mcp": {"lazydb": {"type": "local", "enabled": true}}
        });
        let resolved = resolve(&root).unwrap().unwrap();
        assert_eq!(resolved.effective.format, Format::V1);
        assert!(resolved.shadowed.is_empty());
    }

    #[test]
    fn resolves_native_v2_server() {
        let root = serde_json::json!({
            "mcp": {"servers": {"lazydb": {"type": "local", "disabled": true}}}
        });
        let resolved = resolve(&root).unwrap().unwrap();
        assert_eq!(resolved.effective.format, Format::V2);
        assert_eq!(resolved.effective.format, Format::V2);
    }

    #[test]
    fn native_v2_server_shadows_v1_server() {
        let root = serde_json::json!({
            "mcp": {
                "lazydb": {"type": "local", "enabled": false},
                "servers": {"lazydb": {"type": "local", "disabled": false}}
            }
        });
        let resolved = resolve(&root).unwrap().unwrap();
        assert_eq!(resolved.effective.format, Format::V2);
        assert_eq!(resolved.shadowed.len(), 1);
        assert_eq!(resolved.shadowed[0].format, Format::V1);
    }

    #[test]
    fn rejects_non_object_server_entry() {
        let root = serde_json::json!({"mcp": {"servers": {"lazydb": []}}});
        let error = resolve(&root).unwrap_err().to_string();
        assert!(error.contains("v2 LazyDB entry must be an object"));
    }

    #[test]
    fn missing_server_is_not_an_error() {
        let root = serde_json::json!({"mcp": {"other": {}}});
        assert!(resolve(&root).unwrap().is_none());
    }
}
