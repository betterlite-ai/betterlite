mod tests;

use betterlite_secret_managers::secret_lookup::text_to_bool;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use crate::sections::settings::JsonValue;

const STORE_AUDIT_LOGS_PATH: &str = "litellm_settings.store_audit_logs";
pub const LITELLM_STORE_AUDIT_LOGS_ENV: &str = "LITELLM_STORE_AUDIT_LOGS";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    pub litellm_settings: Global,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct Global {
    #[serde(default)]
    pub store_audit_logs: JsonValue,
    #[serde(default)]
    pub audit_log_callbacks: Vec<String>,
    #[serde(default)]
    pub s3_audit_callback_params: JsonValue,
}

#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub store_audit_logs: Option<bool>,
    pub store_audit_logs_env: Option<bool>,
    pub audit_log_callbacks: Vec<String>,
    pub s3_audit_callback_params: Option<JsonValue>,
}

impl Settings {
    // is_audit_logging_enabled: an explicit argument, then the setting, then the environment, then on
    #[must_use]
    pub fn is_audit_logging_enabled(&self, explicit: Option<bool>) -> bool {
        explicit
            .or(self.store_audit_logs)
            .or(self.store_audit_logs_env)
            .unwrap_or(true)
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            store_audit_logs: configured_flag(self.litellm_settings.store_audit_logs, lookup)?,
            // get_secret_bool reads only true and false after trimming and folding case; other text is unset
            store_audit_logs_env: lookup
                .lookup(LITELLM_STORE_AUDIT_LOGS_ENV)
                .as_deref()
                .and_then(text_to_bool),
            audit_log_callbacks: self.litellm_settings.audit_log_callbacks,
            s3_audit_callback_params: match self.litellm_settings.s3_audit_callback_params {
                JsonValue::Null => None,
                value => Some(value),
            },
        })
    }
}

fn configured_flag<L>(value: JsonValue, lookup: &L) -> Result<Option<bool>, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        JsonValue::Null => Ok(None),
        JsonValue::Bool(flag) => Ok(Some(flag)),
        JsonValue::String(text) => {
            // a string setting resolves an os.environ reference before the same boolean text conversion
            let resolved = resolve_string(text, STORE_AUDIT_LOGS_PATH, lookup)?;
            Ok(text_to_bool(&resolved))
        }
        _ => Err(ConfigError::InvalidScalar {
            path: STORE_AUDIT_LOGS_PATH.to_owned(),
            value: false,
        }),
    }
}
