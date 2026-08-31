//! Installation ownership and state primitives used by the update command.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use clap::ValueEnum;
use reqwest::Client;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use xz2::read::XzDecoder;

const INSTALLATION_SCHEMA: u32 = 1;
const CHANNEL_MANIFEST_SCHEMA: u32 = 1;
const DEFAULT_CHANNEL_BASE_URL: &str = "https://lazydb.yelog.org/channels";
const GITHUB_HOST: &str = "github.com";
const PAGES_HOST: &str = "lazydb.yelog.org";
const SUPPORTED_TARGETS: [&str; 4] = [
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
];

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    #[default]
    Stable,
    Beta,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct UpdateReport {
    pub schema: u32,
    pub manager: InstallationManager,
    pub channel: UpdateChannel,
    pub current_version: Option<String>,
    pub target_version: Option<String>,
    pub status: String,
    pub action: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChannelManifest {
    pub schema: u32,
    pub product: String,
    pub channel: UpdateChannel,
    pub version: String,
    pub tag: String,
    pub prerelease: bool,
    pub published_at: String,
    pub release_url: String,
    pub assets: BTreeMap<String, ManifestAsset>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestAsset {
    pub url: String,
    pub sha256: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("invalid channel manifest JSON: {0}")]
    Json(String),
    #[error("invalid channel manifest: {0}")]
    Invalid(String),
}

pub fn parse_channel_manifest(input: &str) -> Result<ChannelManifest, ManifestError> {
    let manifest: ChannelManifest =
        serde_json::from_str(input).map_err(|error| ManifestError::Json(error.to_string()))?;
    validate_channel_manifest(manifest)
}

fn validate_channel_manifest(manifest: ChannelManifest) -> Result<ChannelManifest, ManifestError> {
    if manifest.schema != CHANNEL_MANIFEST_SCHEMA || manifest.product != "lazydb" {
        return Err(ManifestError::Invalid(
            "unsupported schema or product".into(),
        ));
    }
    let version = Version::parse(&manifest.version)
        .map_err(|_| ManifestError::Invalid("version is not valid SemVer".into()))?;
    let is_beta = !version.pre.is_empty();
    let beta_version = version
        .pre
        .to_string()
        .split_once('.')
        .is_some_and(|(name, number)| {
            name == "beta"
                && number.parse::<u64>().is_ok_and(|number| number > 0)
                && !number.starts_with('0')
                && number.chars().all(|character| character.is_ascii_digit())
        });
    if is_beta != (manifest.channel == UpdateChannel::Beta)
        || manifest.prerelease != is_beta
        || manifest.tag != format!("v{}", manifest.version)
        || (manifest.channel == UpdateChannel::Beta && !beta_version)
    {
        return Err(ManifestError::Invalid(
            "channel, version, tag, or prerelease mismatch".into(),
        ));
    }
    if manifest.published_at.trim().is_empty()
        || !approved_url(&manifest.release_url, GITHUB_HOST)
        || manifest.release_url
            != format!(
                "https://github.com/yelog/lazydb/releases/tag/{}",
                manifest.tag
            )
    {
        return Err(ManifestError::Invalid(
            "invalid release URL or publication timestamp".into(),
        ));
    }
    if manifest.assets.len() != SUPPORTED_TARGETS.len()
        || !SUPPORTED_TARGETS
            .iter()
            .all(|target| manifest.assets.contains_key(*target))
    {
        return Err(ManifestError::Invalid(
            "target asset set is incomplete or contains extras".into(),
        ));
    }
    for target in SUPPORTED_TARGETS {
        let asset = &manifest.assets[target];
        let expected = format!("lazydb_{}_{}.tar.xz", manifest.version, target);
        if asset.url
            != format!(
                "https://github.com/yelog/lazydb/releases/download/{}/{}",
                manifest.tag, expected
            )
            || asset.sha256.len() != 64
            || !asset
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(ManifestError::Invalid(format!(
                "invalid asset for {target}"
            )));
        }
    }
    Ok(manifest)
}

fn approved_url(value: &str, host: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| url.scheme() == "https" && url.host_str() == Some(host))
}

#[async_trait]
pub trait UpdateHttpClient {
    async fn get(&self, url: &str) -> anyhow::Result<String>;

    async fn download(&self, url: &str) -> anyhow::Result<Vec<u8>>;
}

pub struct SystemUpdateHttpClient {
    client: Client,
}

impl Default for SystemUpdateHttpClient {
    fn default() -> Self {
        Self {
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    if attempt.previous().len() >= 10 {
                        return attempt.error("too many HTTP redirects");
                    }
                    let requested = attempt
                        .previous()
                        .first()
                        .expect("redirect attempts have a previous URL");
                    if validate_response_url(requested.as_str(), attempt.url()).is_ok() {
                        attempt.follow()
                    } else {
                        attempt.error("HTTP redirect left the approved host or scheme")
                    }
                }))
                .build()
                .expect("valid HTTP client configuration"),
        }
    }
}

fn validate_response_url(requested: &str, response: &url::Url) -> anyhow::Result<()> {
    let requested = url::Url::parse(requested)?;
    let response_host = response
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("redirected URL has no host"))?;
    let requested_host = requested
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("requested URL has no host"))?;
    let same_host = response_host == requested_host;
    let github_asset_redirect = requested_host == GITHUB_HOST
        && matches!(
            response_host,
            "release-assets.githubusercontent.com" | "objects.githubusercontent.com"
        );
    let local_fixture = matches!(requested_host, "127.0.0.1" | "localhost" | "fixture");
    let valid_scheme = response.scheme() == "https"
        || (local_fixture && requested.scheme() == "http" && response.scheme() == "http");
    if !valid_scheme || !(same_host || github_asset_redirect) {
        anyhow::bail!("HTTP redirect left the approved host or scheme")
    }
    Ok(())
}

#[async_trait]
impl UpdateHttpClient for SystemUpdateHttpClient {
    async fn get(&self, url: &str) -> anyhow::Result<String> {
        let response = self.client.get(url).send().await?.error_for_status()?;
        validate_response_url(url, response.url())?;
        Ok(response.text().await?)
    }

    async fn download(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        let response = self.client.get(url).send().await?.error_for_status()?;
        validate_response_url(url, response.url())?;
        Ok(response.bytes().await?.to_vec())
    }
}

pub trait InstallationStateSource {
    fn state(&self) -> Option<InstallationState>;
}

pub trait UpdateFileSystem {
    fn read_to_string(&self, path: &Path) -> std::io::Result<String>;
}

pub struct SystemUpdateFileSystem;

impl UpdateFileSystem for SystemUpdateFileSystem {
    fn read_to_string(&self, path: &Path) -> std::io::Result<String> {
        fs::read_to_string(path)
    }
}

pub struct InstallationStateFileSource<F = SystemUpdateFileSystem> {
    path: PathBuf,
    file_system: F,
}

impl<F: UpdateFileSystem> InstallationStateSource for InstallationStateFileSource<F> {
    fn state(&self) -> Option<InstallationState> {
        self.file_system
            .read_to_string(&self.path)
            .ok()
            .and_then(|input| parse_installation_state(&input).ok())
    }
}

pub type SystemInstallationStateSource = InstallationStateFileSource<SystemUpdateFileSystem>;

pub const UPDATE_STATUSES: [&str; 5] = [
    "up_to_date",
    "update_available",
    "updated",
    "manager_action_required",
    "error",
];

pub async fn run(args: crate::cli::UpdateArgs, _config: Option<PathBuf>) -> anyhow::Result<String> {
    let paths = crate::persistence::paths::AppPaths::discover().ok();
    let source = SystemInstallationStateSource {
        path: paths
            .as_ref()
            .map(|paths| paths.data_dir.join("install.json"))
            .unwrap_or_else(|| PathBuf::from("install.json")),
        file_system: SystemUpdateFileSystem,
    };
    let mut report = inspect_local_installation(
        args.channel,
        args.allow_downgrade,
        &source,
        &SystemInstallationProbe,
        &SystemUpdateHttpClient::default(),
    )
    .await;
    if !args.check
        && report.manager == InstallationManager::Native
        && report.status == "update_available"
    {
        let state = source
            .state()
            .ok_or_else(|| anyhow::anyhow!("native installation state is unavailable"))?;
        let target = current_target(Some(&state))
            .ok_or_else(|| anyhow::anyhow!("current target is unsupported"))?;
        let manifest = fetch_manifest(report.channel, &SystemUpdateHttpClient::default())
            .await
            .map_err(anyhow::Error::msg)?;
        let lock = UpdateLock::acquire(&native_data_dir(&state)?)?;
        let version = apply_native_update(
            &state,
            &target,
            &manifest,
            &SystemUpdateHttpClient::default(),
        )
        .await?;
        drop(lock);
        report.target_version = Some(version.clone());
        report.current_version = Some(version);
        report.status = "updated".to_owned();
        report.action = None;
    }
    if args.json {
        return Ok(serde_json::to_string(&report)?);
    }
    Ok(format_update_report(&report))
}

async fn inspect_local_installation<P, S, H>(
    requested_channel: Option<UpdateChannel>,
    allow_downgrade: bool,
    source: &S,
    probe: &P,
    http: &H,
) -> UpdateReport
where
    P: InstallationProbe,
    S: InstallationStateSource,
    H: UpdateHttpClient,
{
    let executable = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("lazydb"));
    let state = source.state();
    let manager = detect_installation_manager(&executable, state.as_ref(), probe);
    let channel = resolve_channel(
        requested_channel,
        state.as_ref().map(|state| state.channel.as_str()),
    );
    if manager != InstallationManager::Native {
        return UpdateReport {
            schema: 1,
            manager,
            channel,
            current_version: None,
            target_version: None,
            status: if manager == InstallationManager::Unknown {
                "error".to_owned()
            } else {
                "manager_action_required".to_owned()
            },
            action: match manager {
                InstallationManager::Homebrew | InstallationManager::Npm => {
                    manager_action(manager, channel)
                }
                InstallationManager::Deb => {
                    Some("use your Debian package manager to upgrade lazydb".to_owned())
                }
                InstallationManager::Rpm => {
                    Some("use your RPM package manager to upgrade lazydb".to_owned())
                }
                InstallationManager::Arch => {
                    Some("use your Arch package manager to upgrade lazydb".to_owned())
                }
                InstallationManager::Cargo => Some("cargo install lazydb".to_owned()),
                InstallationManager::Unknown => {
                    Some("installation manager could not be determined".to_owned())
                }
                InstallationManager::Native => None,
            },
        };
    }
    let current_version = state.as_ref().map(|state| state.version.clone());
    let target = current_target(state.as_ref());
    let manifest = fetch_manifest(channel, http).await;
    let (target_version, status, action) = match (manifest, target) {
        (Ok(manifest), Some(target)) if manifest.assets.contains_key(&target) => {
            let target_version = manifest.version.clone();
            let status =
                version_status(current_version.as_deref(), &target_version, allow_downgrade);
            let action = match manager {
                InstallationManager::Native if status == "update_available" => {
                    Some(format!("native target {target}; update will be applied"))
                }
                InstallationManager::Native => None,
                _ => manager_action(manager, channel),
            };
            (Some(target_version), status.to_owned(), action)
        }
        (Err(error), _) => (None, "error".to_owned(), Some(error)),
        (Ok(_), Some(_)) | (_, None) => (
            None,
            "error".to_owned(),
            Some("current target is unsupported".to_owned()),
        ),
    };
    let (status, action) = match manager {
        InstallationManager::Native => (status, action),
        manager @ (InstallationManager::Homebrew | InstallationManager::Npm) => (
            "manager_action_required".to_owned(),
            manager_action(manager, channel),
        ),
        InstallationManager::Deb => (
            "manager_action_required".to_owned(),
            Some("use your Debian package manager to upgrade lazydb".to_owned()),
        ),
        InstallationManager::Rpm => (
            "manager_action_required".to_owned(),
            Some("use your RPM package manager to upgrade lazydb".to_owned()),
        ),
        InstallationManager::Arch => (
            "manager_action_required".to_owned(),
            Some("use your Arch package manager to upgrade lazydb".to_owned()),
        ),
        InstallationManager::Cargo => (
            "manager_action_required".to_owned(),
            Some("cargo install lazydb".to_owned()),
        ),
        InstallationManager::Unknown => (
            "error".to_owned(),
            Some("installation manager could not be determined".to_owned()),
        ),
    };
    UpdateReport {
        schema: 1,
        manager,
        channel,
        current_version,
        target_version,
        status,
        action,
    }
}

fn current_target(state: Option<&InstallationState>) -> Option<String> {
    state
        .map(|state| state.target.clone())
        .or_else(|| {
            Some(
                match (std::env::consts::ARCH, std::env::consts::OS) {
                    ("x86_64", "macos") => "x86_64-apple-darwin",
                    ("aarch64", "macos") => "aarch64-apple-darwin",
                    ("x86_64", "linux") => "x86_64-unknown-linux-gnu",
                    ("aarch64", "linux") => "aarch64-unknown-linux-gnu",
                    _ => "unsupported",
                }
                .to_owned(),
            )
        })
        .and_then(|target| {
            SUPPORTED_TARGETS
                .contains(&target.as_str())
                .then_some(target)
        })
}

fn version_status(current: Option<&str>, target: &str, allow_downgrade: bool) -> &'static str {
    let Some(current) = current.and_then(|version| Version::parse(version).ok()) else {
        return "error";
    };
    let Ok(target) = Version::parse(target) else {
        return "error";
    };
    match target.cmp(&current) {
        std::cmp::Ordering::Greater => "update_available",
        std::cmp::Ordering::Equal => "up_to_date",
        std::cmp::Ordering::Less if allow_downgrade => "update_available",
        std::cmp::Ordering::Less => "up_to_date",
    }
}

async fn fetch_manifest<H: UpdateHttpClient>(
    channel: UpdateChannel,
    http: &H,
) -> Result<ChannelManifest, String> {
    let base = std::env::var("LAZYDB_CHANNEL_BASE_URL")
        .unwrap_or_else(|_| DEFAULT_CHANNEL_BASE_URL.to_owned());
    let base_url = url::Url::parse(&base).map_err(|error| error.to_string())?;
    let local_fixture = matches!(
        base_url.host_str(),
        Some("127.0.0.1") | Some("localhost") | Some("fixture")
    );
    if !(base_url.scheme() == "https" && base_url.host_str() == Some(PAGES_HOST)
        || local_fixture && base_url.scheme() == "http")
    {
        return Err("channel base URL must use HTTPS".to_owned());
    }
    let url = format!(
        "{}/{}.json",
        base.trim_end_matches('/'),
        channel_name(channel)
    );
    let manifest =
        parse_channel_manifest(&http.get(&url).await.map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    (manifest.channel == channel)
        .then_some(manifest)
        .ok_or_else(|| "manifest channel mismatch".to_owned())
}

fn channel_name(channel: UpdateChannel) -> &'static str {
    match channel {
        UpdateChannel::Stable => "stable",
        UpdateChannel::Beta => "beta",
    }
}

fn persisted_channel(channel: &str) -> Option<UpdateChannel> {
    match channel {
        "stable" => Some(UpdateChannel::Stable),
        "beta" => Some(UpdateChannel::Beta),
        _ => None,
    }
}

fn resolve_channel(
    requested_channel: Option<UpdateChannel>,
    persisted: Option<&str>,
) -> UpdateChannel {
    requested_channel
        .or_else(|| persisted.and_then(persisted_channel))
        .unwrap_or_default()
}

fn manager_action(manager: InstallationManager, _channel: UpdateChannel) -> Option<String> {
    match manager {
        InstallationManager::Homebrew => Some("brew upgrade yelog/tap/lazydb".to_owned()),
        InstallationManager::Npm => Some(
            "official npm distribution is unavailable; use the Pages installer or Homebrew"
                .to_owned(),
        ),
        _ => None,
    }
}

fn format_update_report(report: &UpdateReport) -> String {
    let action = report.action.as_deref().unwrap_or("none");
    format!(
        "lazydb update: {} (manager: {:?}, channel: {:?}, action: {action})",
        report.status, report.manager, report.channel
    )
}

struct UpdateLock {
    path: PathBuf,
}

impl UpdateLock {
    fn acquire(data_dir: &Path) -> anyhow::Result<Self> {
        let path = data_dir.join(".install.lock");
        fs::create_dir_all(data_dir)?;
        match fs::create_dir(&path) {
            Ok(()) => {
                fs::write(path.join("pid"), std::process::id().to_string())?;
                Ok(Self { path })
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(anyhow::anyhow!("another lazydb update is in progress"))
            }
            Err(error) => Err(error.into()),
        }
    }
}

impl Drop for UpdateLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn native_data_dir(state: &InstallationState) -> anyhow::Result<PathBuf> {
    let link = fs::read_link(&state.path)
        .map_err(|error| anyhow::anyhow!("native executable is not a symlink: {error}"))?;
    let resolved = if link.is_absolute() {
        link
    } else {
        state
            .path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(link)
    };
    let current = resolved
        .parent()
        .filter(|path| path.file_name().is_some_and(|name| name == "current"))
        .ok_or_else(|| anyhow::anyhow!("native executable does not point through current"))?;
    Ok(current
        .parent()
        .ok_or_else(|| anyhow::anyhow!("native installation root is unavailable"))?
        .to_owned())
}

async fn apply_native_update<H: UpdateHttpClient>(
    state: &InstallationState,
    target: &str,
    manifest: &ChannelManifest,
    http: &H,
) -> anyhow::Result<String> {
    let asset = manifest
        .assets
        .get(target)
        .ok_or_else(|| anyhow::anyhow!("current target is unsupported"))?;
    let archive = http.download(&asset.url).await?;
    let actual = format!("{:x}", Sha256::digest(&archive));
    if actual != asset.sha256 {
        anyhow::bail!("checksum mismatch")
    }

    let data_dir = native_data_dir(state)?;
    let temp_name = format!(".update-{}", unique_suffix());
    let staging = data_dir.join(&temp_name);
    let release = staging.join("release");
    fs::create_dir_all(&release)?;
    if let Err(error) = extract_archive(&archive, &release, manifest, target) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    let binary = release.join("lazydb");
    let reported = match run_staged_version(&binary) {
        Ok(version) => version,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    if reported != manifest.version {
        let _ = fs::remove_dir_all(&staging);
        anyhow::bail!(
            "staged binary reported version {reported}, expected {}",
            manifest.version
        )
    }

    let releases = data_dir.join("releases");
    fs::create_dir_all(&releases)?;
    let destination = releases.join(&manifest.version);
    let destination_created = !destination.exists();
    if !destination_created {
        fs::remove_dir_all(&staging)?;
    } else {
        if let Err(error) = fs::rename(&release, &destination) {
            let _ = fs::remove_dir_all(&staging);
            return Err(error.into());
        }
        fs::remove_dir_all(&staging)?;
    }
    if let Err(error) = publish_native_state(state, &data_dir, &destination, manifest, target) {
        if destination_created {
            let _ = fs::remove_dir_all(&destination);
        }
        return Err(error);
    }
    Ok(manifest.version.clone())
}

fn extract_archive(
    archive: &[u8],
    destination: &Path,
    manifest: &ChannelManifest,
    target: &str,
) -> anyhow::Result<()> {
    let expected_root = format!("lazydb_{}_{}", manifest.version, target);
    let mut decoder = XzDecoder::new(Cursor::new(archive));
    let mut tar_bytes = Vec::new();
    decoder.read_to_end(&mut tar_bytes)?;
    let mut archive = tar::Archive::new(Cursor::new(&tar_bytes));
    let entries = archive.entries()?.collect::<Result<Vec<_>, _>>()?;
    let mut names = std::collections::BTreeSet::new();
    let binary_name = format!("{expected_root}/lazydb");
    let mut root_seen = false;
    let mut binary_seen = false;
    for entry in &entries {
        let name = entry.path()?.to_string_lossy().into_owned();
        let safe = archive_entry_path_is_normal(&name)
            && (name == expected_root || name.starts_with(&format!("{expected_root}/")))
            && names.insert(name.clone());
        if !safe
            || entry.header().entry_type().is_symlink()
            || entry.header().entry_type().is_hard_link()
            || !(entry.header().entry_type().is_dir() || entry.header().entry_type().is_file())
        {
            anyhow::bail!("unsafe archive entry: {name}")
        }
        if name == expected_root && entry.header().entry_type().is_dir() {
            root_seen = true;
        }
        if name == binary_name {
            binary_seen = true;
        }
    }
    if !root_seen || !binary_seen {
        anyhow::bail!("archive does not contain the expected executable")
    }
    let mut archive = tar::Archive::new(Cursor::new(&tar_bytes));
    archive.unpack(destination)?;
    fs::rename(
        destination.join(&expected_root),
        destination.join("lazydb-root"),
    )?;
    let root = destination.join("lazydb-root");
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let path = entry.path();
        let target_path = destination.join(entry.file_name());
        fs::rename(path, target_path)?;
    }
    fs::remove_dir(&root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(destination.join("lazydb"))?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(destination.join("lazydb"), permissions)?;
    }
    Ok(())
}

fn archive_entry_path_is_normal(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('\0')
        && !name.contains('\\')
        && !name.contains(':')
        && Path::new(name)
            .components()
            .all(|component| matches!(component, Component::Normal(part) if !part.is_empty()))
        && !name
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
}

fn run_staged_version(binary: &Path) -> anyhow::Result<String> {
    let output = Command::new(binary).args(["version", "--json"]).output()?;
    if !output.status.success() {
        anyhow::bail!("staged binary failed version check")
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    value["version"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("staged binary returned invalid version JSON"))
}

fn publish_native_state(
    state: &InstallationState,
    data_dir: &Path,
    destination: &Path,
    manifest: &ChannelManifest,
    target: &str,
) -> anyhow::Result<()> {
    let current = data_dir.join("current");
    let current_new = data_dir.join(".current.new");
    let current_old = data_dir.join(".current.old");
    let state_new = data_dir.join(".install.json.new");
    let state_path = data_dir.join("install.json");
    let state_old = data_dir.join(".install.json.old");
    #[cfg(unix)]
    std::os::unix::fs::symlink(destination, &current_new)?;
    let new_state = InstallationState {
        schema: 1,
        product: "lazydb".into(),
        manager: InstallationManager::Native,
        channel: channel_name(manifest.channel).into(),
        version: manifest.version.clone(),
        target: target.into(),
        path: state.path.clone(),
        bin_dir: state.bin_dir.clone(),
        installed_at: state.installed_at.clone(),
    };
    let mut file = fs::File::create(&state_new)?;
    serde_json::to_writer_pretty(&mut file, &new_state)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    if current.exists() || fs::symlink_metadata(&current).is_ok() {
        fs::rename(&current, &current_old)?;
    }
    if let Err(error) = fs::rename(&current_new, &current) {
        let _ = fs::rename(&current_old, &current);
        return Err(error.into());
    }
    if (state_path.exists() || fs::symlink_metadata(&state_path).is_ok())
        && let Err(error) = fs::rename(&state_path, &state_old)
    {
        let _ = fs::remove_file(&current);
        let _ = fs::rename(&current_old, &current);
        return Err(error.into());
    }
    if let Err(error) = fs::rename(&state_new, &state_path) {
        let _ = fs::remove_file(&current);
        let _ = fs::rename(&current_old, &current);
        let _ = fs::rename(&state_old, &state_path);
        return Err(error.into());
    }
    let _ = fs::remove_file(&current_old);
    let _ = fs::remove_file(&state_old);
    Ok(())
}

fn unique_suffix() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string()
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct InstallationState {
    pub schema: u32,
    pub product: String,
    pub manager: InstallationManager,
    pub channel: String,
    pub version: String,
    pub target: String,
    pub path: PathBuf,
    #[serde(default)]
    pub bin_dir: Option<PathBuf>,
    #[serde(default)]
    pub installed_at: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InstallationManager {
    Native,
    Homebrew,
    Npm,
    Deb,
    Rpm,
    Arch,
    Cargo,
    Unknown,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StateError {
    #[error("invalid installation state JSON: {0}")]
    Json(String),
    #[error("unsupported installation state schema: {0}")]
    UnsupportedSchema(u32),
    #[error("installation state describes {product}, not lazydb")]
    WrongProduct { product: String },
    #[error("installation state manager must be native")]
    NotNative,
}

pub fn parse_installation_state(input: &str) -> Result<InstallationState, StateError> {
    let state: InstallationState =
        serde_json::from_str(input).map_err(|error| StateError::Json(error.to_string()))?;
    validate_installation_state(state)
}

pub fn read_installation_state(path: &Path) -> Result<InstallationState, StateError> {
    let input = fs::read_to_string(path).map_err(|error| StateError::Json(error.to_string()))?;
    parse_installation_state(&input)
}

fn validate_installation_state(state: InstallationState) -> Result<InstallationState, StateError> {
    if state.schema != INSTALLATION_SCHEMA {
        return Err(StateError::UnsupportedSchema(state.schema));
    }
    if state.product != "lazydb" {
        return Err(StateError::WrongProduct {
            product: state.product,
        });
    }
    if state.manager != InstallationManager::Native {
        return Err(StateError::NotNative);
    }
    Ok(state)
}

pub trait InstallationProbe {
    fn canonicalize(&self, path: &Path) -> Option<PathBuf>;
    fn read_link(&self, path: &Path) -> Option<PathBuf>;
    fn command_output(&self, command: &str, args: &[&str]) -> Option<String>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemInstallationProbe;

impl InstallationProbe for SystemInstallationProbe {
    fn canonicalize(&self, path: &Path) -> Option<PathBuf> {
        fs::canonicalize(path).ok()
    }

    fn read_link(&self, path: &Path) -> Option<PathBuf> {
        fs::read_link(path).ok()
    }

    fn command_output(&self, command: &str, args: &[&str]) -> Option<String> {
        let output = Command::new(command).args(args).output().ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }
}

pub fn native_installation_is_active<P: InstallationProbe>(
    state: &InstallationState,
    executable: &Path,
    probe: &P,
) -> bool {
    if state.manager != InstallationManager::Native || state.product != "lazydb" {
        return false;
    }
    let Some(state_path) = probe.canonicalize(&state.path) else {
        return false;
    };
    let Some(executable_path) = probe.canonicalize(executable) else {
        return false;
    };
    if state_path != executable_path {
        return false;
    }

    // The visible binary must be a link through `current`, rather than merely
    // resembling a native installation path.
    probe.read_link(executable).is_some_and(|target| {
        let target_has_current = target
            .components()
            .any(|component| component.as_os_str() == "current");
        let resolved_target = if target.is_absolute() {
            target
        } else {
            executable
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(target)
        };
        target_has_current
            && probe
                .canonicalize(&resolved_target)
                .is_some_and(|current| current == executable_path)
    })
}

pub fn detect_installation_manager<P: InstallationProbe>(
    executable: &Path,
    state: Option<&InstallationState>,
    probe: &P,
) -> InstallationManager {
    if let Some(state) = state
        && native_installation_is_active(state, executable, probe)
    {
        return InstallationManager::Native;
    }

    let path = executable.to_string_lossy();
    if path.contains("/Cellar/") || brew_prefix_contains(executable, probe) {
        return InstallationManager::Homebrew;
    }
    if path.contains("node_modules") || npm_prefix_contains(executable, probe) {
        return InstallationManager::Npm;
    }
    if package_owned(executable, probe, "dpkg", "-S") {
        return InstallationManager::Deb;
    }
    if package_owned(executable, probe, "rpm", "-qf") {
        return InstallationManager::Rpm;
    }
    if package_owned(executable, probe, "pacman", "-Qo") {
        return InstallationManager::Arch;
    }
    if cargo_path(executable) {
        return InstallationManager::Cargo;
    }
    InstallationManager::Unknown
}

fn brew_prefix_contains<P: InstallationProbe>(path: &Path, probe: &P) -> bool {
    probe
        .command_output("brew", &["--prefix"])
        .is_some_and(|prefix| path.starts_with(prefix))
}

fn npm_prefix_contains<P: InstallationProbe>(path: &Path, probe: &P) -> bool {
    probe
        .command_output("npm", &["config", "get", "prefix"])
        .is_some_and(|prefix| path.starts_with(Path::new(&prefix)))
}

fn package_owned<P: InstallationProbe>(path: &Path, probe: &P, command: &str, query: &str) -> bool {
    probe
        .command_output(command, &[query, &path.to_string_lossy()])
        .is_some_and(|output| !output.is_empty())
}

fn cargo_path(path: &Path) -> bool {
    let path = path.to_string_lossy();
    path.contains("/.cargo/bin/")
        || path.contains("/target/debug/")
        || path.contains("/target/release/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;
    use xz2::write::XzEncoder;

    #[derive(Default)]
    struct FakeProbe {
        paths: HashMap<PathBuf, PathBuf>,
        commands: HashMap<(String, Vec<String>), String>,
    }

    impl InstallationProbe for FakeProbe {
        fn canonicalize(&self, path: &Path) -> Option<PathBuf> {
            self.paths
                .get(path)
                .cloned()
                .or_else(|| Some(path.to_owned()))
        }

        fn read_link(&self, path: &Path) -> Option<PathBuf> {
            (path.to_string_lossy().contains("/bin/lazydb"))
                .then(|| PathBuf::from("/tmp/data/current/lazydb"))
        }

        fn command_output(&self, command: &str, args: &[&str]) -> Option<String> {
            self.commands
                .get(&(
                    command.to_owned(),
                    args.iter().map(|arg| (*arg).to_owned()).collect(),
                ))
                .cloned()
        }
    }

    fn valid_state(path: &str) -> String {
        format!(
            r#"{{"schema":1,"product":"lazydb","manager":"native","channel":"stable","version":"1.2.3","target":"x86_64-apple-darwin","path":"{path}"}}"#
        )
    }

    #[test]
    fn parses_task_two_native_state() {
        let state = parse_installation_state(&valid_state("/tmp/bin/lazydb")).unwrap();
        assert_eq!(state.manager, InstallationManager::Native);
        assert_eq!(state.version, "1.2.3");
    }

    #[test]
    fn rejects_malformed_missing_and_unsupported_state() {
        assert!(matches!(
            parse_installation_state("{"),
            Err(StateError::Json(_))
        ));
        assert!(matches!(
            parse_installation_state(
                r#"{"schema":2,"product":"lazydb","manager":"native","channel":"stable","version":"1","target":"x","path":"/tmp/lazydb"}"#
            ),
            Err(StateError::UnsupportedSchema(2))
        ));
        assert!(matches!(
            parse_installation_state(r#"{"schema":1,"product":"lazydb","manager":"native"}"#),
            Err(StateError::Json(_))
        ));
        assert!(matches!(
            parse_installation_state(
                r#"{"schema":1,"product":"lazydb","manager":"homebrew","channel":"stable","version":"1","target":"x","path":"/tmp/lazydb"}"#
            ),
            Err(StateError::NotNative)
        ));
    }

    #[test]
    fn stale_native_metadata_does_not_authorize_replacement() {
        let state = parse_installation_state(&valid_state("/tmp/bin/lazydb")).unwrap();
        let probe = FakeProbe::default();
        assert!(!native_installation_is_active(
            &state,
            Path::new("/tmp/other/lazydb"),
            &probe
        ));
    }

    #[test]
    fn confirms_native_current_symlink() {
        let dir = tempdir().unwrap();
        let data = dir.path().join("data");
        let bin = dir.path().join("bin/lazydb");
        let release = data.join("releases/1.2.3/lazydb");
        let state = InstallationState {
            schema: 1,
            product: "lazydb".into(),
            manager: InstallationManager::Native,
            channel: "stable".into(),
            version: "1.2.3".into(),
            target: "x86_64-apple-darwin".into(),
            path: bin.clone(),
            bin_dir: None,
            installed_at: None,
        };
        let mut probe = FakeProbe::default();
        probe.paths.insert(bin.clone(), release.clone());
        probe
            .paths
            .insert(PathBuf::from("/tmp/data/current/lazydb"), release.clone());
        assert!(native_installation_is_active(&state, &bin, &probe));
    }

    #[test]
    fn detects_manager_paths_and_package_probes() {
        let mut probe = FakeProbe::default();
        assert_eq!(
            detect_installation_manager(
                Path::new("/opt/homebrew/Cellar/lazydb/1/bin/lazydb"),
                None,
                &probe
            ),
            InstallationManager::Homebrew
        );
        assert_eq!(
            detect_installation_manager(
                Path::new("/usr/local/lib/node_modules/@yelog/lazydb/bin/lazydb"),
                None,
                &probe
            ),
            InstallationManager::Npm
        );
        assert_eq!(
            detect_installation_manager(Path::new("/work/target/release/lazydb"), None, &probe),
            InstallationManager::Cargo
        );
        assert_eq!(
            detect_installation_manager(Path::new("/opt/lazydb"), None, &probe),
            InstallationManager::Unknown
        );
        probe.commands.insert(
            ("dpkg".into(), vec!["-S".into(), "/opt/lazydb".into()]),
            "lazydb: /opt/lazydb".into(),
        );
        assert_eq!(
            detect_installation_manager(Path::new("/opt/lazydb"), None, &probe),
            InstallationManager::Deb
        );

        probe
            .commands
            .remove(&("dpkg".into(), vec!["-S".into(), "/opt/lazydb".into()]));
        probe.commands.insert(
            ("rpm".into(), vec!["-qf".into(), "/opt/lazydb".into()]),
            "lazydb-1.2.3".into(),
        );
        assert_eq!(
            detect_installation_manager(Path::new("/opt/lazydb"), None, &probe),
            InstallationManager::Rpm
        );

        probe
            .commands
            .remove(&("rpm".into(), vec!["-qf".into(), "/opt/lazydb".into()]));
        probe.commands.insert(
            ("pacman".into(), vec!["-Qo".into(), "/opt/lazydb".into()]),
            "lazydb owns /opt/lazydb".into(),
        );
        assert_eq!(
            detect_installation_manager(Path::new("/opt/lazydb"), None, &probe),
            InstallationManager::Arch
        );
    }

    #[test]
    fn update_report_serializes_the_stable_contract_on_one_line() {
        let report = UpdateReport {
            schema: 1,
            manager: InstallationManager::Homebrew,
            channel: UpdateChannel::Stable,
            current_version: Some("1.2.3".into()),
            target_version: None,
            status: "manager_action_required".into(),
            action: Some("brew upgrade yelog/tap/lazydb".into()),
        };
        let output = serde_json::to_string(&report).unwrap();
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert!(!output.contains('\n'));
        assert_eq!(value["schema"], 1);
        assert_eq!(value["manager"], "homebrew");
        assert_eq!(value["channel"], "stable");
        assert_eq!(value["status"], "manager_action_required");
        assert_eq!(value["action"], "brew upgrade yelog/tap/lazydb");
    }

    #[test]
    fn update_status_contract_includes_all_planned_states() {
        assert_eq!(
            UPDATE_STATUSES,
            [
                "up_to_date",
                "update_available",
                "updated",
                "manager_action_required",
                "error"
            ]
        );
    }

    #[test]
    fn manager_actions_are_explicit_and_npm_distribution_is_unavailable() {
        assert_eq!(
            manager_action(InstallationManager::Homebrew, UpdateChannel::Stable),
            Some("brew upgrade yelog/tap/lazydb".into())
        );
        assert_eq!(
            manager_action(InstallationManager::Npm, UpdateChannel::Stable),
            Some(
                "official npm distribution is unavailable; use the Pages installer or Homebrew"
                    .into()
            )
        );
        assert_eq!(
            manager_action(InstallationManager::Npm, UpdateChannel::Beta),
            Some(
                "official npm distribution is unavailable; use the Pages installer or Homebrew"
                    .into()
            )
        );
    }

    #[test]
    fn persisted_channel_yields_to_explicit_override() {
        assert_eq!(resolve_channel(None, Some("beta")), UpdateChannel::Beta);
        assert_eq!(
            resolve_channel(Some(UpdateChannel::Stable), Some("beta")),
            UpdateChannel::Stable
        );
        assert_eq!(resolve_channel(None, None), UpdateChannel::Stable);
    }

    fn manifest_fixture() -> String {
        let assets = SUPPORTED_TARGETS
            .iter()
            .map(|target| {
                (
                    (*target).to_owned(),
                    ManifestAsset {
                        url: format!(
                            "https://github.com/yelog/lazydb/releases/download/v1.3.0/lazydb_1.3.0_{target}.tar.xz"
                        ),
                        sha256: "a".repeat(64),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        serde_json::to_string(&ChannelManifest {
            schema: 1,
            product: "lazydb".into(),
            channel: UpdateChannel::Stable,
            version: "1.3.0".into(),
            tag: "v1.3.0".into(),
            prerelease: false,
            published_at: "2026-08-31T00:00:00Z".into(),
            release_url: "https://github.com/yelog/lazydb/releases/tag/v1.3.0".into(),
            assets,
        })
        .unwrap()
    }

    #[test]
    fn validates_manifest_fixture_and_rejects_unapproved_asset_url() {
        assert!(parse_channel_manifest(&manifest_fixture()).is_ok());
        let mut value: serde_json::Value = serde_json::from_str(&manifest_fixture()).unwrap();
        value["assets"][SUPPORTED_TARGETS[0]]["url"] =
            serde_json::Value::String("http://evil.invalid/archive.tar.xz".into());
        assert!(matches!(
            parse_channel_manifest(&value.to_string()),
            Err(ManifestError::Invalid(_))
        ));
    }

    #[test]
    fn validates_semver_and_default_downgrade_policy() {
        assert_eq!(
            version_status(Some("1.2.3"), "1.3.0", false),
            "update_available"
        );
        assert_eq!(version_status(Some("1.3.0"), "1.2.3", false), "up_to_date");
        assert_eq!(
            version_status(Some("1.3.0"), "1.2.3", true),
            "update_available"
        );
        assert_eq!(version_status(Some("not-semver"), "1.2.3", false), "error");
    }

    #[test]
    fn archive_paths_require_normal_components_and_expected_root() {
        assert!(archive_entry_path_is_normal("root/lazydb"));
        for path in [
            "root/./lazydb",
            "root//lazydb",
            "root/../lazydb",
            "root\\lazydb",
            "C:\\lazydb",
            "/root/lazydb",
        ] {
            assert!(
                !archive_entry_path_is_normal(path),
                "accepted unsafe path {path}"
            );
        }
    }

    #[test]
    fn archive_without_expected_root_directory_is_rejected() {
        let dir = tempdir().unwrap();
        let target = SUPPORTED_TARGETS[0];
        let manifest = update_manifest("1.3.0", "a".repeat(64));
        let root = format!("lazydb_1.3.0_{target}");
        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            let contents = b"binary";
            let mut header = tar::Header::new_gnu();
            header.set_path(format!("{root}/lazydb")).unwrap();
            header.set_size(contents.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append(&header, &contents[..]).unwrap();
            builder.finish().unwrap();
        }
        let mut compressed = Vec::new();
        let mut encoder = XzEncoder::new(&mut compressed, 6);
        encoder.write_all(&tar_bytes).unwrap();
        encoder.finish().unwrap();

        assert!(
            extract_archive(&compressed, &dir.path().join("release"), &manifest, target,).is_err()
        );
    }

    #[test]
    fn redirected_urls_must_remain_on_approved_scheme_and_host() {
        assert!(
            validate_response_url(
                "https://github.com/yelog/lazydb/archive.tar.xz",
                &url::Url::parse("https://release-assets.githubusercontent.com/archive").unwrap(),
            )
            .is_ok()
        );
        assert!(
            validate_response_url(
                "https://github.com/yelog/lazydb/archive.tar.xz",
                &url::Url::parse("https://evil.example/archive").unwrap(),
            )
            .is_err()
        );
        assert!(
            validate_response_url(
                "https://github.com/yelog/lazydb/archive.tar.xz",
                &url::Url::parse("http://github.com/archive").unwrap(),
            )
            .is_err()
        );
    }

    struct FakeHttp {
        archive: Vec<u8>,
    }

    #[async_trait]
    impl UpdateHttpClient for FakeHttp {
        async fn get(&self, _url: &str) -> anyhow::Result<String> {
            unreachable!("manifest fetch is not part of apply_native_update tests")
        }

        async fn download(&self, _url: &str) -> anyhow::Result<Vec<u8>> {
            Ok(self.archive.clone())
        }
    }

    fn update_manifest(version: &str, digest: String) -> ChannelManifest {
        let target = SUPPORTED_TARGETS[0];
        let mut assets = BTreeMap::new();
        for supported in SUPPORTED_TARGETS {
            assets.insert(
                supported.to_owned(),
                ManifestAsset {
                    url: format!("https://github.com/yelog/lazydb/releases/download/v{version}/lazydb_{version}_{supported}.tar.xz"),
                    sha256: if supported == target { digest.clone() } else { "a".repeat(64) },
                },
            );
        }
        ChannelManifest {
            schema: 1,
            product: "lazydb".into(),
            channel: UpdateChannel::Stable,
            version: version.into(),
            tag: format!("v{version}"),
            prerelease: false,
            published_at: "2026-08-31T00:00:00Z".into(),
            release_url: format!("https://github.com/yelog/lazydb/releases/tag/v{version}"),
            assets,
        }
    }

    fn archive_for(version: &str, target: &str, output: &Path) -> Vec<u8> {
        archive_for_reported(version, version, target, output)
    }

    fn archive_for_reported(
        version: &str,
        reported_version: &str,
        target: &str,
        output: &Path,
    ) -> Vec<u8> {
        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            let root = format!("lazydb_{version}_{target}");
            let mut directory = tar::Header::new_gnu();
            directory.set_path(&root).unwrap();
            directory.set_entry_type(tar::EntryType::Directory);
            directory.set_size(0);
            directory.set_mode(0o755);
            directory.set_cksum();
            builder.append(&directory, &[][..]).unwrap();
            let script = format!(
                "#!/bin/sh\nprintf '{{\"version\":\"{reported_version}\",\"cli_api\":1}}\\n'\n"
            );
            let mut header = tar::Header::new_gnu();
            header.set_path(format!("{root}/lazydb")).unwrap();
            header.set_size(script.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append(&header, script.as_bytes()).unwrap();
            builder.finish().unwrap();
        }
        let mut compressed = Vec::new();
        let mut encoder = XzEncoder::new(&mut compressed, 6);
        encoder.write_all(&tar_bytes).unwrap();
        encoder.finish().unwrap();
        File::create(output)
            .unwrap()
            .write_all(&compressed)
            .unwrap();
        compressed
    }

    #[tokio::test]
    async fn applies_verified_release_and_preserves_previous_release() {
        let dir = tempdir().unwrap();
        let data = dir.path().join("data");
        let old = data.join("releases/1.2.3");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("lazydb"), b"old").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&old, data.join("current")).unwrap();
        let bin = dir.path().join("bin/lazydb");
        fs::create_dir_all(bin.parent().unwrap()).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(data.join("current/lazydb"), &bin).unwrap();
        let state = InstallationState {
            schema: 1,
            product: "lazydb".into(),
            manager: InstallationManager::Native,
            channel: "stable".into(),
            version: "1.2.3".into(),
            target: SUPPORTED_TARGETS[0].into(),
            path: bin,
            bin_dir: None,
            installed_at: None,
        };
        let archive_path = dir.path().join("archive.tar.xz");
        let archive = archive_for("1.3.0", SUPPORTED_TARGETS[0], &archive_path);
        let digest = format!("{:x}", Sha256::digest(&archive));
        let manifest = update_manifest("1.3.0", digest);
        let version = apply_native_update(
            &state,
            SUPPORTED_TARGETS[0],
            &manifest,
            &FakeHttp { archive },
        )
        .await
        .unwrap();
        assert_eq!(version, "1.3.0");
        assert!(old.exists());
        assert_eq!(
            fs::read_link(data.join("current")).unwrap(),
            data.join("releases/1.3.0")
        );
        assert!(data.join("releases/1.3.0/lazydb").exists());
        let state_path = data.join("install.json");
        publish_native_state(
            &state,
            &data,
            &data.join("releases/1.3.0"),
            &manifest,
            SUPPORTED_TARGETS[0],
        )
        .unwrap();
        assert_eq!(
            read_installation_state(&state_path).unwrap().version,
            "1.3.0"
        );
        let persisted = read_installation_state(&state_path).unwrap();
        assert_eq!(persisted.channel, "stable");
        assert_eq!(persisted.target, SUPPORTED_TARGETS[0]);
        assert_eq!(persisted.path, state.path);
    }

    #[tokio::test]
    async fn checksum_failure_leaves_current_release_unchanged() {
        let dir = tempdir().unwrap();
        let data = dir.path().join("data");
        let old = data.join("releases/1.2.3");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("marker"), b"old").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&old, data.join("current")).unwrap();
        let bin = dir.path().join("bin/lazydb");
        fs::create_dir_all(bin.parent().unwrap()).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(data.join("current/lazydb"), &bin).unwrap();
        let state = InstallationState {
            schema: 1,
            product: "lazydb".into(),
            manager: InstallationManager::Native,
            channel: "stable".into(),
            version: "1.2.3".into(),
            target: SUPPORTED_TARGETS[0].into(),
            path: bin,
            bin_dir: None,
            installed_at: None,
        };
        let archive = b"not an archive".to_vec();
        let manifest = update_manifest("1.3.0", "0".repeat(64));
        assert!(
            apply_native_update(
                &state,
                SUPPORTED_TARGETS[0],
                &manifest,
                &FakeHttp { archive }
            )
            .await
            .is_err()
        );
        assert_eq!(fs::read_link(data.join("current")).unwrap(), old);
        assert_eq!(fs::read(old.join("marker")).unwrap(), b"old");
    }

    #[tokio::test]
    async fn staged_version_failure_leaves_current_release_unchanged() {
        let dir = tempdir().unwrap();
        let data = dir.path().join("data");
        let old = data.join("releases/1.2.3");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("marker"), b"old").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&old, data.join("current")).unwrap();
        let bin = dir.path().join("bin/lazydb");
        fs::create_dir_all(bin.parent().unwrap()).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(data.join("current/lazydb"), &bin).unwrap();
        let state = InstallationState {
            schema: 1,
            product: "lazydb".into(),
            manager: InstallationManager::Native,
            channel: "stable".into(),
            version: "1.2.3".into(),
            target: SUPPORTED_TARGETS[0].into(),
            path: bin,
            bin_dir: None,
            installed_at: None,
        };
        let archive_path = dir.path().join("archive.tar.xz");
        let archive = archive_for_reported("1.3.0", "1.3.1", SUPPORTED_TARGETS[0], &archive_path);
        let manifest = update_manifest("1.3.0", format!("{:x}", Sha256::digest(&archive)));
        let result = apply_native_update(
            &state,
            SUPPORTED_TARGETS[0],
            &manifest,
            &FakeHttp { archive },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(fs::read_link(data.join("current")).unwrap(), old);
        assert_eq!(fs::read(old.join("marker")).unwrap(), b"old");
        assert!(!data.join("releases/1.3.0").exists());
    }
}
