use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use thiserror::Error;

const LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Debug, Error)]
pub enum ProfileLockError {
    #[error("profile store is busy")]
    Busy,
    #[error("profile lock operation failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Owns a stable sidecar file for the full duration of an exclusive profile
/// mutation. The sidecar must not be removed on drop: doing so can split locks
/// across multiple inodes when another process already has the old file open.
#[derive(Clone)]
pub struct ProfileLock {
    _file: std::sync::Arc<File>,
    path: PathBuf,
}

impl ProfileLock {
    pub fn acquire(profile_path: &Path) -> Result<Self, ProfileLockError> {
        let lock_path = lock_path(profile_path)?;
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)?;
        set_private_file_permissions(&file)?;

        let deadline = Instant::now() + LOCK_TIMEOUT;
        loop {
            match file.try_lock() {
                Ok(()) => {
                    return Ok(Self {
                        _file: std::sync::Arc::new(file),
                        path: lock_path,
                    });
                }
                Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    thread::sleep(RETRY_INTERVAL);
                }
                Err(std::fs::TryLockError::WouldBlock) => return Err(ProfileLockError::Busy),
                Err(std::fs::TryLockError::Error(error)) => {
                    return Err(ProfileLockError::Io(error));
                }
            }
        }
    }

    pub(crate) fn protects(&self, profile_path: &Path) -> Result<bool, ProfileLockError> {
        Ok(self.path == lock_path(profile_path)?)
    }
}

fn lock_path(profile_path: &Path) -> Result<PathBuf, ProfileLockError> {
    let absolute = if profile_path.is_absolute() {
        profile_path.to_owned()
    } else {
        std::env::current_dir()?.join(profile_path)
    };
    let parent = absolute.parent().ok_or_else(|| {
        ProfileLockError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "profile path has no parent directory",
        ))
    })?;
    std::fs::create_dir_all(parent)?;
    let canonical_parent = parent.canonicalize()?;
    let file_name = absolute.file_name().ok_or_else(|| {
        ProfileLockError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "profile path has no file name",
        ))
    })?;
    let path = canonical_parent.join(file_name);
    let target = match std::fs::canonicalize(&path) {
        Ok(target) => target,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => path,
        Err(error) => return Err(ProfileLockError::Io(error)),
    };
    let parent = target.parent().ok_or_else(|| {
        ProfileLockError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "resolved profile path has no parent directory",
        ))
    })?;
    let name = target.file_name().ok_or_else(|| {
        ProfileLockError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "resolved profile path has no file name",
        ))
    })?;
    Ok(parent.join(format!("{}.lock", name.to_string_lossy())))
}

#[cfg(unix)]
fn set_private_file_permissions(file: &File) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_private_file_permissions(_file: &File) -> Result<(), std::io::Error> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        process::{Command, Stdio},
        sync::mpsc,
        thread,
        time::Duration,
    };

    use tempfile::tempdir;

    use super::{ProfileLock, ProfileLockError};

    #[test]
    fn exclusive_profile_lock_is_released_when_guard_drops() {
        let temp = tempdir().unwrap();
        let profile = temp.path().join("connections.toml");
        let first = ProfileLock::acquire(&profile).unwrap();
        assert!(matches!(
            ProfileLock::acquire(&profile),
            Err(ProfileLockError::Busy)
        ));
        drop(first);
        ProfileLock::acquire(&profile).unwrap();
    }

    #[test]
    fn relative_and_symlink_paths_resolve_to_the_same_lock() {
        let temp = tempdir().unwrap();
        let real = temp.path().join("real");
        let alias = temp.path().join("alias");
        fs::create_dir_all(&real).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&real, &alias).unwrap();

        let real_lock = ProfileLock::acquire(&real.join("connections.toml")).unwrap();
        assert!(matches!(
            ProfileLock::acquire(&alias.join("connections.toml")),
            Err(ProfileLockError::Busy)
        ));
        drop(real_lock);
    }

    #[test]
    fn child_process_lock_contention_times_out_without_splitting_sidecar() {
        const CHILD_ENV: &str = "LAZYDB_PROFILE_LOCK_TEST_CHILD";
        if let Some(path) = std::env::var_os(CHILD_ENV) {
            let _guard = ProfileLock::acquire(PathBuf::from(path).as_path()).unwrap();
            println!("LOCK_READY");
            thread::sleep(Duration::from_secs(30));
            return;
        }

        use std::path::PathBuf;
        let temp = tempdir().unwrap();
        let profile = temp.path().join("connections.toml");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("persistence::profile_transaction::tests::child_process_lock_contention_times_out_without_splitting_sidecar")
            .arg("--nocapture")
            .env(CHILD_ENV, &profile)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let mut lines = BufReader::new(stdout).lines();
            while let Some(Ok(line)) = lines.next() {
                if line == "LOCK_READY" {
                    let _ = sender.send(());
                    break;
                }
            }
        });
        receiver.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(
            ProfileLock::acquire(&profile),
            Err(ProfileLockError::Busy)
        ));
        child.kill().unwrap();
        child.wait().unwrap();
        ProfileLock::acquire(&profile).unwrap();
    }
}
