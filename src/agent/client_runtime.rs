//! Runtime identification for client-specific diagnostics.

use std::{
    process::{Command, Stdio},
    time::Duration,
};

use semver::Version;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OpenCodeRuntime {
    V1,
    V2,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OpenCodeVersion {
    pub version: Option<Version>,
    pub runtime: OpenCodeRuntime,
    pub detail: Option<String>,
}

pub(crate) fn parse_version(output: &str) -> OpenCodeVersion {
    let token = output.split_whitespace().find_map(|token| {
        let token = token.trim_start_matches('v');
        Version::parse(token).ok()
    });
    let runtime = match token.as_ref().map(|version| version.major) {
        Some(1) => OpenCodeRuntime::V1,
        Some(2) => OpenCodeRuntime::V2,
        _ => OpenCodeRuntime::Unknown,
    };
    OpenCodeVersion {
        version: token,
        runtime,
        detail: None,
    }
}

pub(crate) fn detect(program: &str, timeout: Duration) -> OpenCodeVersion {
    let Ok(mut child) = Command::new(program)
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    else {
        return OpenCodeVersion {
            version: None,
            runtime: OpenCodeRuntime::Unknown,
            detail: Some(format!("could not execute {program} within the timeout")),
        };
    };
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = child.wait_with_output().ok();
                let text = output
                    .as_ref()
                    .map(|output| String::from_utf8_lossy(&output.stdout))
                    .unwrap_or_default();
                let mut parsed = parse_version(&text);
                if !status.success() {
                    parsed.detail = Some(format!("{program} exited with {status}"));
                }
                if parsed.version.is_none() && parsed.detail.is_none() {
                    parsed.detail = Some(format!("{program} returned an unrecognized version"));
                }
                return parsed;
            }
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return OpenCodeVersion {
                    version: None,
                    runtime: OpenCodeRuntime::Unknown,
                    detail: Some(format!("could not execute {program} within the timeout")),
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_v1_and_v2_versions() {
        assert_eq!(
            parse_version("opencode v1.9.4").runtime,
            OpenCodeRuntime::V1
        );
        assert_eq!(
            parse_version("opencode v2.0.2").runtime,
            OpenCodeRuntime::V2
        );
    }

    #[test]
    fn ignores_unrelated_output_and_unknown_versions() {
        assert_eq!(
            parse_version("warning\nopencode v2.0.2\n").version,
            Some(Version::new(2, 0, 2))
        );
        assert_eq!(
            parse_version("development build").runtime,
            OpenCodeRuntime::Unknown
        );
    }

    #[test]
    fn failed_program_is_reported_as_unknown() {
        let result = detect("definitely-not-an-opencode-binary", Duration::from_secs(1));
        assert_eq!(result.runtime, OpenCodeRuntime::Unknown);
        assert!(result.detail.is_some());
    }
}
