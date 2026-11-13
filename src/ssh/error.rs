use crate::db::{DatabaseError, ErrorCategory};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SshErrorKind {
    Unavailable,
    Unsupported,
    Configuration,
    HostKeyUnknown,
    HostKeyChanged,
    InteractionRequired,
    Authentication,
    Network,
    Timeout,
    Bind,
    Destination,
    Exited,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct SshError {
    pub kind: SshErrorKind,
    pub message: &'static str,
}

impl SshError {
    pub(crate) const fn new(kind: SshErrorKind, message: &'static str) -> Self {
        Self { kind, message }
    }

    pub fn code(&self) -> &'static str {
        match self.kind {
            SshErrorKind::Unavailable => "ssh_unavailable",
            SshErrorKind::Unsupported => "ssh_unsupported",
            SshErrorKind::Configuration => "ssh_configuration",
            SshErrorKind::HostKeyUnknown => "ssh_host_key_unknown",
            SshErrorKind::HostKeyChanged => "ssh_host_key_changed",
            SshErrorKind::InteractionRequired => "ssh_interaction_required",
            SshErrorKind::Authentication => "ssh_authentication_failed",
            SshErrorKind::Network => "ssh_network",
            SshErrorKind::Timeout => "ssh_timeout",
            SshErrorKind::Bind => "ssh_forward_conflict",
            SshErrorKind::Destination => "ssh_destination_unreachable",
            SshErrorKind::Exited => "ssh_tunnel_exited",
            SshErrorKind::Cancelled => "ssh_cancelled",
        }
    }

    pub(crate) fn cancelled() -> Self {
        Self::new(
            SshErrorKind::Cancelled,
            "SSH tunnel connection was cancelled",
        )
    }

    pub(crate) fn timeout() -> Self {
        Self::new(
            SshErrorKind::Timeout,
            "SSH tunnel startup timed out; check the bastion and authentication, or increase the connection timeout",
        )
    }

    pub(crate) fn exited() -> Self {
        Self::new(
            SshErrorKind::Exited,
            "SSH tunnel disconnected; reconnect before running another operation",
        )
    }
}

impl SshError {
    pub fn into_database_error(self) -> DatabaseError {
        let category = match self.kind {
            SshErrorKind::Unavailable
            | SshErrorKind::Unsupported
            | SshErrorKind::Configuration
            | SshErrorKind::HostKeyUnknown
            | SshErrorKind::HostKeyChanged
            | SshErrorKind::InteractionRequired => ErrorCategory::Configuration,
            SshErrorKind::Authentication => ErrorCategory::Authentication,
            SshErrorKind::Cancelled => ErrorCategory::Cancelled,
            _ => ErrorCategory::Network,
        };
        DatabaseError {
            category,
            code: Some(self.code().to_owned()),
            message: self.message.to_owned(),
            diagnostic: None,
        }
    }
}

pub(crate) fn classify_output(output: &str) -> Option<SshError> {
    let lower = output.to_ascii_lowercase();
    let (kind, message) = if lower.contains("remote host identification has changed")
        || lower.contains("offending ")
        || lower.contains("host key for ") && lower.contains("has changed")
    {
        (
            SshErrorKind::HostKeyChanged,
            "SSH host key changed; verify the new fingerprint through a trusted channel before updating known_hosts",
        )
    } else if lower.contains("host key verification failed")
        || lower.contains("no ") && lower.contains("host key is known")
    {
        (
            SshErrorKind::HostKeyUnknown,
            "SSH host key is not trusted; verify the bastion fingerprint with system ssh before connecting",
        )
    } else if lower.contains("bad configuration option")
        || lower.contains("unknown option")
        || lower.contains("unsupported option")
        || lower.contains("illegal option")
    {
        (
            SshErrorKind::Unsupported,
            "The installed SSH client does not support the required OpenSSH options",
        )
    } else if lower.contains("address already in use")
        || lower.contains("cannot listen to port")
        || lower.contains("could not request local forwarding")
        || lower.contains("cannot bind")
    {
        (
            SshErrorKind::Bind,
            "SSH could not bind its private forwarding endpoint",
        )
    } else if lower.contains("read_passphrase")
        || lower.contains("passphrase for key")
        || lower.contains("no tty present")
        || lower.contains("can't open /dev/tty")
    {
        (
            SshErrorKind::InteractionRequired,
            "SSH requires interactive authentication; unlock the key with ssh-add or authenticate with system ssh first",
        )
    } else if lower.contains("permission denied")
        || lower.contains("no more authentication methods")
        || lower.contains("authentication failed")
    {
        (
            SshErrorKind::Authentication,
            "SSH authentication failed; check the SSH user, key and agent, and unlock encrypted keys with ssh-add",
        )
    } else if lower.contains("administratively prohibited")
        || lower.contains("open failed: connect failed")
        || lower.contains("connect_to ")
    {
        (
            SshErrorKind::Destination,
            "The SSH bastion could not reach the database; check its hostname, port and forwarding permissions",
        )
    } else if lower.contains("connection timed out")
        || lower.contains("connection refused")
        || lower.contains("could not resolve hostname")
        || lower.contains("no route to host")
        || lower.contains("connection closed by")
        || lower.contains("connection reset by")
    {
        (
            SshErrorKind::Network,
            "SSH could not connect to the bastion; check the SSH host, port, network and jump configuration",
        )
    } else {
        return None;
    };
    Some(SshError::new(kind, message))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_key_change_is_not_reported_as_unknown_key() {
        let error = classify_output(
            "REMOTE HOST IDENTIFICATION HAS CHANGED!\nHost key verification failed.",
        )
        .unwrap();
        assert_eq!(error.kind, SshErrorKind::HostKeyChanged);
    }

    #[test]
    fn diagnostics_do_not_repeat_remote_or_local_secrets() {
        let error =
            classify_output("Permission denied: hostile\u{1b}[2J user-password private-key-path")
                .unwrap();
        assert!(!error.message.contains("user-password"));
        assert!(!error.message.contains('\u{1b}'));
        assert_eq!(
            error.into_database_error().code.as_deref(),
            Some("ssh_authentication_failed")
        );
    }
}
