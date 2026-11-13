pub mod commands;
pub mod error;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::private_json::files::{PrivateDocument, PrivateFiles};
use tokio::runtime::Builder;
use url::Url;

use error::ConfigError;

pub const HIDDEN_COMMANDS_KEY: &str = "hidden_commands";

const CONFIG_DIRECTORY: &str = ".litellm";
const CONFIG_FILE: &str = "config.json";
const FULL_URL_MESSAGE: &str = "base_url must be a full http:// or https:// URL including a host";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfigKey {
    BaseUrl,
    HiddenCommands,
}

impl ConfigKey {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "base_url" => Some(Self::BaseUrl),
            HIDDEN_COMMANDS_KEY => Some(Self::HiddenCommands),
            _ => None,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::BaseUrl => "base_url",
            Self::HiddenCommands => HIDDEN_COMMANDS_KEY,
        }
    }

    fn normalize(self, value: &str) -> Result<String, ConfigError> {
        match self {
            Self::BaseUrl => normalize_base_url(value),
            Self::HiddenCommands => normalize_hidden_commands(value),
        }
    }
}

pub fn config_file_path(environment: &dyn EnvironmentLookup) -> Option<PathBuf> {
    Some(
        environment
            .system_home()?
            .join(CONFIG_DIRECTORY)
            .join(CONFIG_FILE),
    )
}

pub fn load_config(
    environment: &dyn EnvironmentLookup,
    warnings: &mut dyn Write,
) -> BTreeMap<String, String> {
    let Some(path) = config_file_path(environment) else {
        return BTreeMap::new();
    };
    if !path.exists() {
        return BTreeMap::new();
    }
    let failure = match fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<BTreeMap<String, String>>(&text) {
            Ok(config) => return config,
            Err(source) => source.to_string(),
        },
        Err(source) => source.to_string(),
    };
    // a warning the writer refuses cannot change the outcome: the file still reads as empty
    let _unused = writeln!(
        warnings,
        "Warning: ignoring invalid config file {}: {failure}",
        path.display()
    );
    BTreeMap::new()
}

pub fn save_config(
    config: &BTreeMap<String, String>,
    environment: &dyn EnvironmentLookup,
) -> Result<PathBuf, ConfigError> {
    let path = config_file_path(environment).ok_or(ConfigError::NoHome)?;
    let directory = path.parent().ok_or(ConfigError::NoHome)?;
    let document = PrivateDocument::json(config)?;
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(|source| ConfigError::Runtime { source })?;
    runtime.block_on(async {
        let files = PrivateFiles::default();
        files.ensure_private_dir(directory).await?;
        files.write_private(&path, document).await
    })?;
    Ok(path)
}

pub fn parse_hidden_commands(raw: Option<&str>) -> BTreeSet<String> {
    raw.unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

pub fn hidden_command_names(environment: &dyn EnvironmentLookup) -> BTreeSet<String> {
    let mut warnings = io::sink();
    let stored = load_config(environment, &mut warnings)
        .get(HIDDEN_COMMANDS_KEY)
        .cloned();
    parse_hidden_commands(stored.as_deref())
}

pub fn normalize_base_url(value: &str) -> Result<String, ConfigError> {
    if value.bytes().any(|byte| byte <= 32 || byte == 127) {
        return Err(ConfigError::Usage(
            "base_url must not contain whitespace or control characters".to_owned(),
        ));
    }
    let Ok(parsed) = Url::parse(value) else {
        return Err(ConfigError::Usage(FULL_URL_MESSAGE.to_owned()));
    };
    let after_scheme = value.get(parsed.scheme().len()..).unwrap_or_default();
    let has_authority = after_scheme
        .strip_prefix("://")
        .is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'));
    if !matches!(parsed.scheme(), "http" | "https") || !has_authority || parsed.host_str().is_none()
    {
        return Err(ConfigError::Usage(FULL_URL_MESSAGE.to_owned()));
    }
    if value.contains('?') || value.contains('#') {
        return Err(ConfigError::Usage(
            "base_url must not include a query string or fragment".to_owned(),
        ));
    }
    if has_credentials(value, &parsed) {
        return Err(ConfigError::Usage(
            "base_url must not contain credentials; pass --api-key separately".to_owned(),
        ));
    }
    Ok(value.trim_end_matches('/').to_owned())
}

fn has_credentials(value: &str, parsed: &Url) -> bool {
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return true;
    }
    let Some((_, rest)) = value.split_once("://") else {
        return false;
    };
    rest.split('/')
        .next()
        .is_some_and(|authority| authority.contains('@'))
}

fn normalize_hidden_commands(raw: &str) -> Result<String, ConfigError> {
    let names = parse_hidden_commands(Some(raw));
    if names.is_empty() {
        return Err(ConfigError::Usage(format!(
            "{HIDDEN_COMMANDS_KEY} must be a comma-separated list of command names, e.g. \
             `lite config set {HIDDEN_COMMANDS_KEY} codex,opencode`. To list everything again, \
             run `lite config unset {HIDDEN_COMMANDS_KEY}`"
        )));
    }
    if names.iter().any(|name| name.contains(' ')) {
        return Err(ConfigError::Usage(format!(
            "{HIDDEN_COMMANDS_KEY} entries must be single command names, without spaces"
        )));
    }
    let joined = names.into_iter().collect::<Vec<String>>().join(",");
    Ok(joined)
}

mod tests;
