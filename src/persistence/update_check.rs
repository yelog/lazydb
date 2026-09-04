use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

const SCHEMA: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct UpdateCheckCache {
    pub schema: u32,
    pub checked_at: u64,
    pub channel: String,
    pub target_version: Option<String>,
}

impl UpdateCheckCache {
    pub fn read(path: &Path) -> Option<Self> {
        let cache: Self = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
        (cache.schema == SCHEMA && cache.checked_at > 0).then_some(cache)
    }

    pub fn is_fresh(&self, now: SystemTime, interval: Duration) -> bool {
        let checked = UNIX_EPOCH + Duration::from_secs(self.checked_at);
        now.duration_since(checked).is_ok_and(|age| age <= interval)
    }

    pub fn write_atomic(&self, path: &Path) -> std::io::Result<()> {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let temporary = PathBuf::from(format!("{}.tmp", path.display()));
        let contents = serde_json::to_vec_pretty(self).expect("cache is serializable");
        fs::write(&temporary, contents)?;
        fs::rename(temporary, path)
    }
}

pub fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn missing_and_malformed_cache_is_ignored() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("update-check.json");
        assert!(UpdateCheckCache::read(&path).is_none());
        fs::write(&path, "not json").unwrap();
        assert!(UpdateCheckCache::read(&path).is_none());
    }

    #[test]
    fn cache_round_trips_and_expires() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("state/update-check.json");
        let cache = UpdateCheckCache {
            schema: SCHEMA,
            checked_at: 100,
            channel: "stable".into(),
            target_version: Some("1.2.3".into()),
        };
        cache.write_atomic(&path).unwrap();
        assert_eq!(UpdateCheckCache::read(&path), Some(cache.clone()));
        assert!(cache.is_fresh(
            UNIX_EPOCH + Duration::from_secs(101),
            Duration::from_secs(2)
        ));
        assert!(!cache.is_fresh(
            UNIX_EPOCH + Duration::from_secs(103),
            Duration::from_secs(2)
        ));
    }
}
