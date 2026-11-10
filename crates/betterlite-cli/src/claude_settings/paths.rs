use std::path::{Path, PathBuf, is_separator};

use betterlite_config::loader::EnvironmentLookup;
use sha2::{Digest as _, Sha256};
use soft_canonicalize::soft_canonicalize;

use super::error::ClaudeSettingsError;

pub const CLAUDE_CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeSettingsPaths {
    pub settings: PathBuf,
    pub backup: PathBuf,
    pub autoroute_backup: PathBuf,
    pub configure_state: PathBuf,
    pub configure_state_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsFileOwner {
    pub backup_path: PathBuf,
    pub start_command: &'static str,
    pub stop_command: &'static str,
}

#[must_use]
pub fn claude_settings_paths(home: &Path) -> ClaudeSettingsPaths {
    let litellm = home.join(".litellm");
    ClaudeSettingsPaths {
        settings: home.join(".claude").join("settings.json"),
        backup: litellm.join("claude_settings_backup.json"),
        autoroute_backup: litellm
            .join("autorouter")
            .join("claude_settings_backup.json"),
        configure_state: litellm.join("claude_configure_state.json"),
        configure_state_dir: litellm.join("claude_configure_state"),
    }
}

#[must_use]
pub fn claude_settings_path(environment: &dyn EnvironmentLookup) -> Option<PathBuf> {
    let configured_dir = environment
        .lookup(CLAUDE_CONFIG_DIR_ENV)
        .filter(|dir| !dir.is_empty());
    let Some(config_dir) = configured_dir else {
        return environment
            .system_home()
            .map(|home| claude_settings_paths(&home).settings);
    };
    expand_home(&config_dir, environment).map(|dir| dir.join("settings.json"))
}

pub fn settings_file_owners(
    settings_path: &Path,
    paths: &ClaudeSettingsPaths,
) -> Result<Vec<SettingsFileOwner>, ClaudeSettingsError> {
    if resolve_settings_path(settings_path)? != resolve_settings_path(&paths.settings)? {
        return Ok(Vec::new());
    }
    Ok(vec![
        SettingsFileOwner {
            backup_path: paths.backup.clone(),
            start_command: "lite up",
            stop_command: "lite down",
        },
        SettingsFileOwner {
            backup_path: paths.autoroute_backup.clone(),
            start_command: "lite autoroute start",
            stop_command: "lite autoroute stop",
        },
    ])
}

pub fn configure_state_path(
    settings_path: &Path,
    paths: &ClaudeSettingsPaths,
) -> Result<PathBuf, ClaudeSettingsError> {
    let resolved = resolve_settings_path(settings_path)?;
    if resolved == resolve_settings_path(&paths.settings)? {
        return Ok(paths.configure_state.clone());
    }
    let digest = hex::encode(Sha256::digest(resolved.as_os_str().as_encoded_bytes()));
    Ok(paths.configure_state_dir.join(format!("{digest}.json")))
}

pub fn refuse_while_owned(
    settings_path: &Path,
    owners: &[SettingsFileOwner],
) -> Result<(), ClaudeSettingsError> {
    let Some(owner) = owners.iter().find(|owner| owner.backup_path.exists()) else {
        return Ok(());
    };
    Err(ClaudeSettingsError::Owned {
        path: settings_path.to_owned(),
        backup_path: owner.backup_path.clone(),
        start_command: owner.start_command,
        stop_command: owner.stop_command,
    })
}

pub fn resolve_settings_path(settings_path: &Path) -> Result<PathBuf, ClaudeSettingsError> {
    soft_canonicalize(settings_path).map_err(|source| ClaudeSettingsError::Resolve {
        path: settings_path.to_owned(),
        source,
    })
}

fn expand_home(value: &str, environment: &dyn EnvironmentLookup) -> Option<PathBuf> {
    let Some(rest) = value.strip_prefix('~') else {
        return Some(PathBuf::from(value));
    };
    if !(rest.is_empty() || rest.starts_with(is_separator)) {
        return Some(PathBuf::from(value));
    }
    let home = environment.system_home()?;
    Some(home.join(rest.trim_start_matches(is_separator)))
}

mod tests;
