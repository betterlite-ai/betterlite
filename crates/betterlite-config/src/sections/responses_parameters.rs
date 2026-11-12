use betterlite_models::call_helpers::drop_params::{drop_params_env_flag, drop_params_flag};
use betterlite_models::decisions::Json;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLiteLlm,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlm {
    #[serde(default)]
    drop_params: Option<Json>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    pub drop_params: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let environment = environment_flag(lookup);
        let drop_params = match self.litellm_settings.drop_params {
            None | Some(Json::Null) => environment,
            Some(value) => yaml_flag(&value),
        };
        Ok(Settings { drop_params })
    }
}

fn environment_flag<L>(lookup: &L) -> bool
where
    L: EnvironmentLookup,
{
    let configured = lookup.lookup("LITELLM_DROP_PARAMS");
    let flag = drop_params_env_flag(configured.as_deref());
    if flag.warned {
        let text = configured.as_deref().unwrap_or_default().trim();
        tracing::warn!(
            "LITELLM_DROP_PARAMS={} is not a flag value, treating it as on. Set it to true or false",
            quoted(text)
        );
    }
    flag.enabled
}

fn yaml_flag(value: &Json) -> bool {
    let flag = drop_params_flag(Some(value));
    if flag.warned {
        tracing::warn!(
            "litellm_settings.drop_params={} is not a flag value, treating it as off",
            flag_text(value)
        );
    }
    flag.enabled
}

fn quoted(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('\'', "\\'");
    format!("'{escaped}'")
}

fn flag_text(value: &Json) -> String {
    match value {
        Json::Text(text) => quoted(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            serde_json::to_string(value).unwrap_or_default()
        }
    }
}
