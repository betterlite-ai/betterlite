use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::files::{PrivateDocument, PrivateFiles};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use soft_canonicalize::soft_canonicalize;
use thiserror::Error;
use tokio::fs::OpenOptions;
use tokio::time;

use crate::agent_launch::codex_config_path;
use crate::claude_settings::error::ClaudeSettingsError;
use crate::claude_settings::paths;
use crate::codex_settings::error::CodexSettingsError;
use crate::codex_settings::paths::codex_configure_state_path;
use crate::config::{error::ConfigError, normalize_base_url};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

pub const SAVED_SETUP_VERSION: u8 = 1;

type Error = AgentSetupProfilesError;
type SetupResult<T> = Result<T, Error>;

mod tests;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
#[derive(strum::Display)]
#[strum(serialize_all = "lowercase")]
#[rustfmt::skip]
pub enum Target { Claude, Codex }

#[derive(Clone)]
#[rustfmt::skip]
pub struct SavedSetup { pub target: Target, pub settings_path: String, pub base_url: String, pub api_key: SecretString, pub model: Option<String> }

impl SavedSetup {
    #[rustfmt::skip]
    pub fn new(target: Target, settings_path: String, base_url: String, api_key: SecretString, model: Option<String>) -> SetupResult<Self> {
        let normalized = normalize_base_url(&base_url).map_err(Error::BaseUrlInvalid)?;
        if normalized.strip_suffix("/v1").unwrap_or(&normalized) != base_url {
            return Err(Error::BaseUrlNotNormalized);
        }
        let key = api_key.expose_secret();
        if key.is_empty() || key.chars().any(|chr| chr <= ' ' || chr == '\u{7f}') {
            return Err(Error::ApiKeyInvalid);
        }
        let bad_model = model.as_deref().is_some_and(|value| {
            value.is_empty() || value.chars().any(|character| character.is_ascii_control())
        });
        if bad_model {
            return Err(Error::ModelInvalid);
        }
        Ok(Self { target, settings_path, base_url, api_key, model })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[rustfmt::skip]
struct StoredSetup {
    #[serde(default = "default_version")] version: u8, target: Target,
    settings_path: String, base_url: String, api_key: SecretString,
    #[expect(clippy::option_option, reason = "a missing model key is invalid while a null one is allowed, so the reader keeps all three states")]
    #[serde(default, with = "serde_with::rust::double_option")] model: Option<Option<String>>,
}

#[rustfmt::skip]
const fn default_version() -> u8 { SAVED_SETUP_VERSION }

#[derive(Serialize)]
#[rustfmt::skip]
struct SetupDocument<'saved> {
    version: u8, target: Target, settings_path: &'saved str,
    base_url: &'saved str, api_key: &'saved str, model: Option<&'saved str>,
}

#[derive(Debug, Error)]
#[rustfmt::skip]
pub enum AgentSetupProfilesError {
    #[error("Gateway must be normalized")] BaseUrlNotNormalized,
    #[error("Invalid gateway URL: {0}")] BaseUrlInvalid(#[source] ConfigError),
    #[error("Invalid virtual key")] ApiKeyInvalid,
    #[error("Invalid model choice")] ModelInvalid,
    #[error("the environment has no home directory for the {0} setup")] Path(Target),
    #[error("could not resolve the {0} settings path: {1}")] ClaudeSettings(Target, #[source] ClaudeSettingsError),
    #[error("could not resolve the {0} settings path: {1}")] CodexSettings(Target, #[source] CodexSettingsError),
    #[error("could not resolve the {0} settings path {1}: {2}")] ResolveSettings(Target, PathBuf, #[source] io::Error),
    #[error("Could not read saved {0} setup at {1}; no settings were changed")] Read(Target, PathBuf, #[source] io::Error),
    #[error("Saved {0} setup at {1} is invalid or unsupported. Run `lite unconfigure {0} --forget` to discard it; no settings were changed")] InvalidJson(Target, PathBuf, #[source] serde_json::Error),
    #[error("Saved {0} setup at {1} is invalid or unsupported. Run `lite unconfigure {0} --forget` to discard it; no settings were changed")] InvalidValues(Target, PathBuf, #[source] Box<Self>),
    #[error("Saved {0} setup at {1} is invalid or unsupported. Run `lite unconfigure {0} --forget` to discard it; no settings were changed")] InvalidPath(Target, PathBuf, #[source] io::Error),
    #[error("Saved {0} setup at {1} is invalid or unsupported. Run `lite unconfigure {0} --forget` to discard it; no settings were changed")] Invalid(Target, PathBuf),
    #[error("Could not lock agent setup; retry when other configure commands finish")] LockPrepare(PathBuf, #[source] PrivateJsonError),
    #[error("Could not lock agent setup; retry when other configure commands finish")] LockIo(PathBuf, #[source] io::Error),
    #[error("Could not lock agent setup; retry when other configure commands finish")] Locked,
    #[error("Could not save {0} setup; no {0} settings were changed")] Save(Target, #[source] PrivateJsonError),
    #[error("Could not remove saved {0} setup at {1}")] Forget(Target, PathBuf, #[source] io::Error),
}

#[rustfmt::skip]
pub trait Clock: Send + Sync { fn now(&self) -> Instant; }

#[rustfmt::skip]
pub struct SystemClock;

#[rustfmt::skip]
impl Clock for SystemClock { fn now(&self) -> Instant { Instant::now() } }

#[must_use]
#[rustfmt::skip]
pub struct SetupLocksGuard { _files: Vec<File> } // dropping these closes each descriptor, releasing its OS-level lock

pub fn settings_path_for(target: Target, environment: &dyn EnvironmentLookup) -> Option<PathBuf> {
    match target {
        Target::Claude => paths::claude_settings_path(environment),
        Target::Codex => codex_config_path(environment),
    }
}

#[rustfmt::skip]
pub fn receipt_path_for(target: Target, settings_path: &Path, environment: &dyn EnvironmentLookup) -> SetupResult<PathBuf> {
    match target {
        Target::Claude => {
            let configured = environment.system_home();
            let home = configured.ok_or(Error::Path(target))?;
            paths::configure_state_path(settings_path, &paths::claude_settings_paths(&home))
                .map_err(|source| Error::ClaudeSettings(target, source))
        }
        Target::Codex => codex_configure_state_path(settings_path)
            .map_err(|source| Error::CodexSettings(target, source)),
    }
}

#[rustfmt::skip]
pub fn setup_profile_path(target: Target, settings_path: &Path, environment: &dyn EnvironmentLookup) -> SetupResult<PathBuf> {
    let receipt = receipt_path_for(target, settings_path, environment)?;
    let stem = receipt.file_stem().unwrap_or_else(|| "".as_ref());
    let mut name = stem.to_os_string();
    name.push("_profile.json");
    Ok(receipt.with_file_name(name))
}

#[rustfmt::skip]
pub fn read_saved_setup(target: Target, environment: &dyn EnvironmentLookup) -> SetupResult<Option<SavedSetup>> {
    let settings_path = settings_path_for(target, environment).ok_or(Error::Path(target))?;
    let path = setup_profile_path(target, &settings_path, environment)?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(Error::Read(target, path, source)),
    };
    let stored: StoredSetup = serde_json::from_slice(&bytes)
        .map_err(|source| Error::InvalidJson(target, path.clone(), source))?;
    if stored.version != SAVED_SETUP_VERSION || stored.target != target {
        return Err(Error::Invalid(target, path));
    }
    let Some(model) = stored.model else {
        return Err(Error::Invalid(target, path));
    };
    let saved = SavedSetup::new(
        target,
        stored.settings_path,
        stored.base_url,
        stored.api_key,
        model,
    )
    .map_err(|source| Error::InvalidValues(target, path.clone(), Box::new(source)))?;
    let live = soft_canonicalize(&settings_path)
        .map_err(|source| Error::InvalidPath(target, path.clone(), source))?;
    let recorded = soft_canonicalize(&saved.settings_path)
        .map_err(|source| Error::InvalidPath(target, path.clone(), source))?;
    if recorded != live || (target == Target::Codex && saved.model.is_none()) {
        return Err(Error::Invalid(target, path));
    }
    Ok(Some(saved))
}

#[rustfmt::skip]
pub async fn save_setup(saved: &SavedSetup, environment: &(dyn EnvironmentLookup + Send + Sync)) -> SetupResult<()> {
    let settings_path =
        settings_path_for(saved.target, environment).ok_or(Error::Path(saved.target))?;
    let path = setup_profile_path(saved.target, &settings_path, environment)?;
    let parent = path.parent().ok_or(Error::Path(saved.target))?;
    let document = SetupDocument {
        version: SAVED_SETUP_VERSION,
        target: saved.target,
        settings_path: &saved.settings_path,
        base_url: &saved.base_url,
        api_key: saved.api_key.expose_secret(),
        model: saved.model.as_deref(),
    };
    let files = PrivateFiles::default();
    let written = async {
        files.ensure_private_dir(parent).await?;
        let private = PrivateDocument::json(&document)?;
        files.write_private(&path, private).await
    }
    .await;
    written.map_err(|source| Error::Save(saved.target, source))
}

pub fn forget_saved_setup(target: Target, environment: &dyn EnvironmentLookup) -> SetupResult<()> {
    let settings_path = settings_path_for(target, environment).ok_or(Error::Path(target))?;
    let path = setup_profile_path(target, &settings_path, environment)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(Error::Forget(target, path, source)),
    }
}

pub fn lock_path_for(target: Target, environment: &dyn EnvironmentLookup) -> SetupResult<PathBuf> {
    let configured = environment.system_home();
    let home = configured.ok_or(Error::Path(target))?;
    let settings_path = settings_path_for(target, environment).ok_or(Error::Path(target))?;
    let resolved = soft_canonicalize(&settings_path)
        .map_err(|source| Error::ResolveSettings(target, settings_path.clone(), source))?;
    let mut digest = Sha256::new();
    digest.update(target.to_string().as_bytes());
    digest.update(b":");
    digest.update(resolved.as_os_str().as_encoded_bytes());
    let name = format!("{}.lock", hex::encode(digest.finalize()));
    Ok(home.join(".litellm").join("setup-locks").join(name))
}

#[rustfmt::skip]
pub async fn setup_locks(targets: &[Target], environment: &(dyn EnvironmentLookup + Send + Sync), clock: &dyn Clock) -> SetupResult<SetupLocksGuard> {
    let files_access = PrivateFiles::default();
    let mut files = Vec::new();
    for target in BTreeSet::from_iter(targets.iter().copied()) {
        let path = lock_path_for(target, environment)?;
        let file = open_lock_file(&path, &files_access).await?;
        files.push(acquire_lock(&path, file, clock).await?);
    }
    Ok(SetupLocksGuard { _files: files })
}

async fn acquire_lock(path: &Path, file: File, clock: &dyn Clock) -> SetupResult<File> {
    let started = clock.now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(fs::TryLockError::WouldBlock) => {
                let now = clock.now();
                let elapsed = now.saturating_duration_since(started);
                if elapsed >= Duration::from_secs(10) {
                    return Err(Error::Locked);
                }
                time::sleep(Duration::from_millis(10)).await;
            }
            Err(fs::TryLockError::Error(source)) => {
                return Err(Error::LockIo(path.to_owned(), source));
            }
        }
    }
}

async fn open_lock_file(path: &Path, files: &PrivateFiles) -> SetupResult<File> {
    let Some(parent) = path.parent() else {
        return Err(Error::LockIo(
            path.to_owned(),
            io::Error::from(io::ErrorKind::InvalidInput),
        ));
    };
    files
        .ensure_private_dir(parent)
        .await
        .map_err(|source| Error::LockPrepare(path.to_owned(), source))?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options
        .open(path)
        .await
        .map_err(|source| Error::LockIo(path.to_owned(), source))?;
    #[cfg(unix)]
    {
        let metadata = file
            .metadata()
            .await
            .map_err(|source| Error::LockIo(path.to_owned(), source))?;
        if metadata.permissions().mode() & 0o077 != 0 {
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .await
                .map_err(|source| Error::LockIo(path.to_owned(), source))?;
        }
    }
    Ok(file.into_std().await)
}
