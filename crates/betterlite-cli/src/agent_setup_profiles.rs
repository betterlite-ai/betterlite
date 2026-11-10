use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::private_json::files::{PrivateDocument, PrivateFiles};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use soft_canonicalize::soft_canonicalize;

use crate::agent_launch::codex_config_path;
use crate::claude_settings::paths;
use crate::codex_settings::paths::codex_configure_state_path;
use crate::config::normalize_base_url;

pub mod error;
pub mod locks;

use error::AgentSetupProfilesError;

pub const SAVED_SETUP_VERSION: u8 = 1;

type Error = AgentSetupProfilesError;
type SetupResult<T> = Result<T, Error>;

mod tests;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
#[derive(strum::Display)]
#[strum(serialize_all = "lowercase")]
pub enum Target {
    Claude,
    Codex,
}

#[derive(Clone)]
pub struct SavedSetup {
    pub target: Target,
    pub settings_path: String,
    pub base_url: String,
    pub api_key: SecretString,
    pub model: Option<String>,
}

impl SavedSetup {
    pub fn new(
        target: Target,
        settings_path: String,
        base_url: String,
        api_key: SecretString,
        model: Option<String>,
    ) -> SetupResult<Self> {
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
        Ok(Self {
            target,
            settings_path,
            base_url,
            api_key,
            model,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredSetup {
    #[serde(default = "default_version")]
    version: u8,
    target: Target,
    settings_path: String,
    base_url: String,
    api_key: SecretString,
    #[expect(
        clippy::option_option,
        reason = "a missing model key is invalid while a null one is allowed, so the reader keeps all three states"
    )]
    #[serde(default, with = "serde_with::rust::double_option")]
    model: Option<Option<String>>,
}

const fn default_version() -> u8 {
    SAVED_SETUP_VERSION
}

#[derive(Serialize)]
struct SetupDocument<'saved> {
    version: u8,
    target: Target,
    settings_path: &'saved str,
    base_url: &'saved str,
    api_key: &'saved str,
    model: Option<&'saved str>,
}

pub fn settings_path_for(target: Target, environment: &dyn EnvironmentLookup) -> Option<PathBuf> {
    match target {
        Target::Claude => paths::claude_settings_path(environment),
        Target::Codex => codex_config_path(environment),
    }
}

pub fn receipt_path_for(
    target: Target,
    settings_path: &Path,
    environment: &dyn EnvironmentLookup,
) -> SetupResult<PathBuf> {
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

pub fn setup_profile_path(
    target: Target,
    settings_path: &Path,
    environment: &dyn EnvironmentLookup,
) -> SetupResult<PathBuf> {
    let receipt = receipt_path_for(target, settings_path, environment)?;
    let stem = receipt.file_stem().unwrap_or_else(|| "".as_ref());
    let mut name = stem.to_os_string();
    name.push("_profile.json");
    Ok(receipt.with_file_name(name))
}

pub fn read_saved_setup(
    target: Target,
    environment: &dyn EnvironmentLookup,
) -> SetupResult<Option<SavedSetup>> {
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

pub async fn save_setup(
    saved: &SavedSetup,
    environment: &(dyn EnvironmentLookup + Send + Sync),
) -> SetupResult<()> {
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
