use betterlite_secret_managers::secret_lookup::text_to_bool;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum PlatformValue {
    Null,
    Boolean(bool),
    Number(serde_json::Number),
    Text(String),
    List(Vec<Self>),
    Mapping(IndexMap<String, Self>),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnsupportedParams {
    List(Vec<PlatformValue>),
    Set(Vec<PlatformValue>),
    // the Python adapter spells frozenset without the underscore
    #[serde(rename = "frozenset")]
    FrozenSet(Vec<PlatformValue>),
    Tuple(Vec<PlatformValue>),
    Other(PlatformValue),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlatformInputs {
    pub workspace_id: Option<String>,
    pub aws_workspace_id: Option<String>,
    pub anthropic_hyphen_workspace_id: Option<String>,
    pub anthropic_workspace_id: Option<String>,
    pub unsupported_params: Option<UnsupportedParams>,
}

#[derive(Debug, Clone, Default)]
pub struct PlatformEnvironment {
    pub anthropic_aws_workspace_id: Option<SecretString>,
    pub anthropic_workspace_id: Option<SecretString>,
    pub anthropic_aws_base_url: Option<String>,
    pub anthropic_aws_api_base: Option<String>,
    pub anthropic_aws_api_key: Option<SecretString>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub deployments: Vec<PlatformInputs>,
    pub litellm_settings: PlatformInputs,
    pub environment: PlatformEnvironment,
}

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
    #[serde(default)]
    litellm_settings: RawParams,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawParams,
}

#[derive(Debug, Default, Deserialize)]
struct RawParams {
    workspace_id: Option<String>,
    aws_workspace_id: Option<String>,
    #[serde(rename = "anthropic-workspace-id")]
    anthropic_hyphen_workspace_id: Option<String>,
    anthropic_workspace_id: Option<String>,
    #[serde(rename = "claude_platform_unsupported_params")]
    unsupported_params: Option<PlatformValue>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let deployments = self
            .model_list
            .into_iter()
            .map(|deployment| deployment.litellm_params.into_inputs())
            .collect();
        Ok(Settings {
            deployments,
            litellm_settings: self.litellm_settings.into_inputs(),
            environment: PlatformEnvironment {
                anthropic_aws_workspace_id: secret_str(lookup, "ANTHROPIC_AWS_WORKSPACE_ID"),
                anthropic_workspace_id: secret_str(lookup, "ANTHROPIC_WORKSPACE_ID"),
                anthropic_aws_base_url: lookup.lookup("ANTHROPIC_AWS_BASE_URL"),
                anthropic_aws_api_base: lookup.lookup("ANTHROPIC_AWS_API_BASE"),
                anthropic_aws_api_key: secret_str(lookup, "ANTHROPIC_AWS_API_KEY"),
            },
        })
    }
}

impl RawParams {
    fn into_inputs(self) -> PlatformInputs {
        PlatformInputs {
            workspace_id: self.workspace_id,
            aws_workspace_id: self.aws_workspace_id,
            anthropic_hyphen_workspace_id: self.anthropic_hyphen_workspace_id,
            anthropic_workspace_id: self.anthropic_workspace_id,
            unsupported_params: self.unsupported_params.map(override_of),
        }
    }
}

// a YAML sequence is LiteLLM's list; every other shape is the type the warning names
fn override_of(value: PlatformValue) -> UnsupportedParams {
    match value {
        PlatformValue::List(items) => UnsupportedParams::List(items),
        PlatformValue::Null
        | PlatformValue::Boolean(_)
        | PlatformValue::Number(_)
        | PlatformValue::Text(_)
        | PlatformValue::Mapping(_) => UnsupportedParams::Other(value),
    }
}

// get_secret_str keeps an empty value as a real setting and reads true or false as unset
fn secret_str<L>(lookup: &L, name: &str) -> Option<SecretString>
where
    L: EnvironmentLookup,
{
    let value = lookup.lookup(name)?;
    if text_to_bool(&value).is_some() {
        return None;
    }
    Some(SecretString::from(value))
}
