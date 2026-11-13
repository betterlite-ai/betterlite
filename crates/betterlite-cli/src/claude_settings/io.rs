use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::private_json::files::{PrivateDocument, PrivateFiles};

use super::error::ClaudeSettingsError;
use super::paths::resolve_settings_path;
use super::{ENV_KEY, SettingsDocument};

// Python's Path.resolve gives up after 40 links with ELOOP
const MAX_SYMLINK_HOPS: usize = 40;

pub fn load_json_or_empty(path: &Path) -> Result<SettingsDocument, ClaudeSettingsError> {
    let content = if path.exists() {
        fs::read(path).map_err(|source| ClaudeSettingsError::Read {
            path: path.to_owned(),
            source,
        })?
    } else {
        Vec::new()
    };
    if content
        .iter()
        .all(|byte| byte.is_ascii_whitespace() || *byte == 0x0b)
    {
        return Ok(SettingsDocument::new());
    }
    serde_json::from_slice::<SettingsDocument>(&content).map_err(|source| {
        ClaudeSettingsError::NotObject {
            path: path.to_owned(),
            source,
        }
    })
}

pub fn env_object(
    settings: &SettingsDocument,
    path: &Path,
) -> Result<BTreeMap<String, JsonValue>, ClaudeSettingsError> {
    match settings.get(ENV_KEY) {
        None | Some(JsonValue::Null) => Ok(BTreeMap::new()),
        Some(JsonValue::Object(env)) => Ok(env
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) | JsonValue::Array(_),
        ) => Err(ClaudeSettingsError::NonObjectEnv {
            path: path.to_owned(),
        }),
    }
}

pub fn write_target(settings_path: &Path) -> Result<PathBuf, ClaudeSettingsError> {
    if !settings_path.is_symlink() {
        return Ok(settings_path.to_owned());
    }
    let mut current = settings_path.to_owned();
    for _ in 0..=MAX_SYMLINK_HOPS {
        if !current.is_symlink() {
            return resolve_settings_path(&current);
        }
        current = followed_link(&current, settings_path)?;
    }
    Err(ClaudeSettingsError::Resolve {
        path: settings_path.to_owned(),
        source: io::Error::other("too many levels of symbolic links"),
    })
}

fn followed_link(link: &Path, settings_path: &Path) -> Result<PathBuf, ClaudeSettingsError> {
    let target = fs::read_link(link).map_err(|source| ClaudeSettingsError::Resolve {
        path: settings_path.to_owned(),
        source,
    })?;
    let Some(parent) = link.parent() else {
        return Ok(target);
    };
    if target.is_relative() {
        return Ok(parent.join(target));
    }
    Ok(target)
}

pub async fn write_claude_settings(
    files: &PrivateFiles,
    target: &Path,
    settings: &SettingsDocument,
) -> Result<(), ClaudeSettingsError> {
    let document =
        PrivateDocument::json(settings).map_err(|source| ClaudeSettingsError::Write {
            path: target.to_owned(),
            source,
        })?;
    let staged = files
        .stage_private(target, document)
        .await
        .map_err(|source| ClaudeSettingsError::Write {
            path: target.to_owned(),
            source,
        })?;
    files
        .commit_staged_json(&staged, target)
        .await
        .map_err(|source| ClaudeSettingsError::Write {
            path: target.to_owned(),
            source,
        })
}

mod tests;
