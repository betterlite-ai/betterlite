use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const CREDENTIAL_OVERRIDES_PATH: &str = "litellm_settings.enable_model_config_credential_overrides";
const ENFORCED_PARAMS_PATH: &str = "general_settings.enforced_params";
const SERVICE_ACCOUNT_PATH: &str = "general_settings.service_account_settings";
const SERVICE_ACCOUNT_PARAMS_PATH: &str =
    "general_settings.service_account_settings.enforced_params";
const STALE_ALIAS_BYPASS_VARIABLE: &str = "LITELLM_ENABLE_TEAM_STALE_ALIAS_BYPASS";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLiteLlmSettings,
    #[serde(default)]
    general_settings: RawGeneralSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlmSettings {
    #[serde(rename = "enable_model_config_credential_overrides")]
    credential_overrides: Option<RawFlag>,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneralSettings {
    enforced_params: Option<RawList>,
    service_account_settings: Option<RawServiceAccountSlot>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawServiceAccountSlot {
    Object(RawServiceAccount),
    Other(IgnoredAny),
}

#[derive(Debug, Default, Deserialize)]
struct RawServiceAccount {
    enforced_params: Option<RawList>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Bool(bool),
    Other(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawList {
    Items(Vec<RawParam>),
    Other(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawParam {
    Text(String),
    Other(IgnoredAny),
}

// enable_model_config_credential_overrides is the pinned False default, enforced_params the pinned None
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub credential_overrides: bool,
    pub enforced_params: Option<Vec<String>>,
    pub service_account_params: Option<Vec<String>>,
    pub team_stale_alias_bypass: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let enforced_params =
            text_list(self.general_settings.enforced_params, ENFORCED_PARAMS_PATH)?;
        // LiteLLM caches this flag on first use and keeps False when the variable is unset or empty
        let bypass_text = lookup
            .lookup(STALE_ALIAS_BYPASS_VARIABLE)
            .unwrap_or_default();
        Ok(Settings {
            credential_overrides: credential_flag(self.litellm_settings.credential_overrides)?,
            enforced_params,
            service_account_params: service_account_list(
                self.general_settings.service_account_settings,
            )?,
            team_stale_alias_bypass: betterlite_env::flag(
                STALE_ALIAS_BYPASS_VARIABLE,
                &bypass_text,
            )?
            .unwrap_or(false),
        })
    }
}

fn credential_flag(value: Option<RawFlag>) -> Result<bool, ConfigError> {
    match value {
        None => Ok(false),
        Some(RawFlag::Bool(enabled)) => Ok(enabled),
        Some(RawFlag::Other(_)) => Err(ConfigError::InvalidScalar {
            path: CREDENTIAL_OVERRIDES_PATH.to_owned(),
            value: false,
        }),
    }
}

fn service_account_list(
    value: Option<RawServiceAccountSlot>,
) -> Result<Option<Vec<String>>, ConfigError> {
    match value {
        None => Ok(None),
        Some(RawServiceAccountSlot::Object(account)) => {
            text_list(account.enforced_params, SERVICE_ACCOUNT_PARAMS_PATH)
        }
        Some(RawServiceAccountSlot::Other(_)) => Err(ConfigError::InvalidScalar {
            path: SERVICE_ACCOUNT_PATH.to_owned(),
            value: false,
        }),
    }
}

fn text_list(value: Option<RawList>, path: &str) -> Result<Option<Vec<String>>, ConfigError> {
    match value {
        None => Ok(None),
        Some(RawList::Items(items)) => {
            let mut texts = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    RawParam::Text(text) => texts.push(text),
                    RawParam::Other(_) => {
                        return Err(ConfigError::InvalidScalar {
                            path: path.to_owned(),
                            value: false,
                        });
                    }
                }
            }
            Ok(Some(texts))
        }
        Some(RawList::Other(_)) => Err(ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        }),
    }
}
