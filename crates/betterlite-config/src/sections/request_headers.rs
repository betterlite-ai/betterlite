#![expect(
    clippy::struct_excessive_bools,
    reason = "the settings mirror LiteLLM's independent request header and tracing switches"
)]

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

const ENVIRONMENT_PREFIX: &str = "os.environ/";
const ADD_USER_INFORMATION_PATH: &str = "litellm_settings.add_user_information_to_llm_headers";
const FORWARD_ORG_ID_PATH: &str = "general_settings.forward_openai_org_id";
const FORWARD_HEADERS_PATH: &str = "general_settings.forward_client_headers_to_llm_api";
const FORWARD_TRACEPARENT_PATH: &str = "litellm_settings.forward_traceparent_to_llm_provider";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
    #[serde(default)]
    litellm_settings: RawLitellm,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    user_header_mappings: Option<Vec<UserHeaderMapping>>,
    user_header_name: Option<String>,
    forward_openai_org_id: Option<RawFlag>,
    #[serde(rename = "forward_client_headers_to_llm_api")]
    forward_client_headers: Option<RawFlag>,
}

#[derive(Debug, Default, Deserialize)]
struct RawLitellm {
    #[serde(rename = "add_user_information_to_llm_headers")]
    add_user_information: Option<RawFlag>,
    model_group_settings: Option<RawModelGroupSettings>,
    #[serde(rename = "forward_traceparent_to_llm_provider")]
    forward_traceparent: Option<RawFlag>,
}

#[derive(Debug, Default, Deserialize)]
struct RawModelGroupSettings {
    #[serde(rename = "forward_client_headers_to_llm_api")]
    forward_client_headers: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Boolean(bool),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UserHeaderMapping {
    pub header_name: String,
    pub litellm_user_role: UserHeaderRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserHeaderRole {
    InternalUser,
    Customer,
}

// LiteLLM checks each flag with `is True`, so an unset flag is off
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub user_header_mappings: Vec<UserHeaderMapping>,
    pub user_header_name: Option<String>,
    pub add_user_information: bool,
    pub forward_openai_org_id: bool,
    // model_group_settings.forward_client_headers_to_llm_api: an empty list still grants every model
    pub model_group_forwarding: Option<Vec<String>>,
    pub forward_client_headers: bool,
    pub forward_traceparent: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general = self.general_settings;
        let litellm = self.litellm_settings;
        Ok(Settings {
            user_header_mappings: general.user_header_mappings.unwrap_or_default(),
            user_header_name: general.user_header_name,
            add_user_information: flag(
                litellm.add_user_information,
                ADD_USER_INFORMATION_PATH,
                lookup,
            )?,
            forward_openai_org_id: flag(
                general.forward_openai_org_id,
                FORWARD_ORG_ID_PATH,
                lookup,
            )?,
            model_group_forwarding: litellm
                .model_group_settings
                .and_then(|group| group.forward_client_headers),
            forward_client_headers: flag(
                general.forward_client_headers,
                FORWARD_HEADERS_PATH,
                lookup,
            )?,
            forward_traceparent: flag(
                litellm.forward_traceparent,
                FORWARD_TRACEPARENT_PATH,
                lookup,
            )?,
        })
    }
}

// only an os.environ reference turns text into a bool, and an empty variable counts as unset
fn flag<L>(value: Option<RawFlag>, path: &str, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        None => Ok(false),
        Some(RawFlag::Boolean(enabled)) => Ok(enabled),
        Some(RawFlag::Text(text)) => {
            let Some(variable) = text.strip_prefix(ENVIRONMENT_PREFIX).map(str::to_owned) else {
                return Err(ConfigError::InvalidScalar {
                    path: path.to_owned(),
                    value: false,
                });
            };
            let resolved = resolve_string(text, path, lookup)?;
            Ok(betterlite_env::flag(&variable, &resolved)?.unwrap_or(false))
        }
    }
}
