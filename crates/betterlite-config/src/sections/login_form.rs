use secrecy::SecretString;
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const HIDE_HINT_NAME: &str = "LITELLM_HIDE_DEFAULT_CREDENTIALS_HINT";
const PROXY_BASE_URL_NAME: &str = "PROXY_BASE_URL";
const SERVER_ROOT_PATH_NAME: &str = "SERVER_ROOT_PATH";
const UI_PASSWORD_NAME: &str = "UI_PASSWORD";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(default)]
    hide_default_credentials_hint: Option<LiteralBoolean>,
}

// LiteLLM tests the setting with `is True`, so a string "true" and every other value leave the hint visible
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum LiteralBoolean {
    Boolean(bool),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub proxy_base_url: String,
    pub server_root_path: String,
    pub hide_default_credentials_hint: bool,
    pub hide_hint_environment: String,
    pub ui_password: SecretString,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            proxy_base_url: lookup.lookup(PROXY_BASE_URL_NAME).unwrap_or_default(),
            server_root_path: lookup.lookup(SERVER_ROOT_PATH_NAME).unwrap_or_default(),
            hide_default_credentials_hint: matches!(
                self.general_settings.hide_default_credentials_hint,
                Some(LiteralBoolean::Boolean(true))
            ),
            // LiteLLM reads this variable with a "false" default before it compares the lowercased text
            hide_hint_environment: lookup
                .lookup(HIDE_HINT_NAME)
                .unwrap_or_else(|| "false".to_owned()),
            ui_password: SecretString::from(lookup.lookup(UI_PASSWORD_NAME).unwrap_or_default()),
        })
    }
}
