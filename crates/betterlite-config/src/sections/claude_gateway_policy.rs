use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

const ENABLED_PATH: &str = "general_settings.enable_claude_code_gateway";
const MANAGED_PATH: &str = "general_settings.claude_code_gateway_managed_settings";
const MAX_DEPTH_VARIABLE: &str = "DEFAULT_MAX_RECURSE_DEPTH";
const DEFAULT_MAX_DEPTH: usize = 100;
// LiteLLM starts at the config root, so the managed mapping is entered two levels down
const MANAGED_DEPTH: usize = 2;

pub type ManagedSettings = IndexMap<String, Json>;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    enable_claude_code_gateway: Option<RawFlag>,
    #[serde(rename = "claude_code_gateway_managed_settings")]
    managed_settings: Option<RawManagedSettings>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Boolean(bool),
    Text(String),
    Other(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawManagedSettings {
    Mapping(ManagedSettings),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub managed_settings: Option<ManagedSettings>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general = self.general_settings;
        Ok(Settings {
            enabled: enabled_value(general.enable_claude_code_gateway, lookup)?,
            managed_settings: managed_value(general.managed_settings, lookup)?,
        })
    }
}

fn enabled_value<L>(value: Option<RawFlag>, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    let invalid = || ConfigError::InvalidScalar {
        path: ENABLED_PATH.to_owned(),
        value: false,
    };
    match value {
        None => Ok(false),
        Some(RawFlag::Boolean(flag)) => Ok(flag),
        Some(RawFlag::Text(text)) => {
            let Some(variable) = text.strip_prefix("os.environ/").map(str::to_owned) else {
                return Err(invalid());
            };
            let resolved = resolve_string(text, ENABLED_PATH, lookup)?;
            Ok(betterlite_env::flag(&variable, &resolved)?.unwrap_or(false))
        }
        Some(RawFlag::Other(_)) => Err(invalid()),
    }
}

fn managed_value<L>(
    value: Option<RawManagedSettings>,
    lookup: &L,
) -> Result<Option<ManagedSettings>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(configured) = value else {
        return Ok(None);
    };
    let RawManagedSettings::Mapping(settings) = configured else {
        return Err(ConfigError::InvalidScalar {
            path: MANAGED_PATH.to_owned(),
            value: false,
        });
    };
    let max_depth = recursion_depth(lookup)?;
    Ok(Some(resolved_mapping(
        settings,
        MANAGED_DEPTH,
        max_depth,
        lookup,
    )))
}

fn recursion_depth<L>(lookup: &L) -> Result<usize, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(value) = lookup.lookup(MAX_DEPTH_VARIABLE) else {
        return Ok(DEFAULT_MAX_DEPTH);
    };
    Ok(betterlite_env::number::<usize>(MAX_DEPTH_VARIABLE, &value)?.unwrap_or(DEFAULT_MAX_DEPTH))
}

// LiteLLM resolves mapping values and dicts inside lists, never list strings, up to the depth limit
fn resolved_mapping<L>(
    settings: ManagedSettings,
    depth: usize,
    max_depth: usize,
    lookup: &L,
) -> ManagedSettings
where
    L: EnvironmentLookup,
{
    if depth > max_depth {
        return settings;
    }
    settings
        .into_iter()
        .map(|(key, value)| (key, resolved_entry(value, depth, max_depth, lookup)))
        .collect()
}

fn resolved_entry<L>(value: Json, depth: usize, max_depth: usize, lookup: &L) -> Json
where
    L: EnvironmentLookup,
{
    let child_depth = depth.saturating_add(1);
    match value {
        Json::Object(fields) => {
            Json::Object(resolved_mapping(fields, child_depth, max_depth, lookup))
        }
        Json::List(items) => Json::List(
            items
                .into_iter()
                .map(|item| match item {
                    Json::Object(fields) => {
                        Json::Object(resolved_mapping(fields, child_depth, max_depth, lookup))
                    }
                    other @ (Json::Null
                    | Json::Bool(_)
                    | Json::Number(_)
                    | Json::Text(_)
                    | Json::List(_)) => other,
                })
                .collect(),
        ),
        Json::Text(text) => environment_reference(text, lookup),
        other @ (Json::Null | Json::Bool(_) | Json::Number(_)) => other,
    }
}

fn environment_reference<L>(text: String, lookup: &L) -> Json
where
    L: EnvironmentLookup,
{
    let Some(name) = text.strip_prefix("os.environ/").map(str::to_owned) else {
        return Json::Text(text);
    };
    environment_value(&name, lookup)
}

fn environment_value<L>(name: &str, lookup: &L) -> Json
where
    L: EnvironmentLookup,
{
    let Some(value) = lookup.lookup(name) else {
        return Json::Null;
    };
    match value.trim().to_lowercase().as_str() {
        "true" => Json::Bool(true),
        "false" => Json::Bool(false),
        _ => Json::Text(value),
    }
}
