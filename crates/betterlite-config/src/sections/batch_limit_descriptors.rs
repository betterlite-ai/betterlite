#[path = "batch_limit_descriptors/tests.rs"]
mod tests;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use crate::sections::settings::JsonValue;

const DISABLE_PATH: &str = "general_settings.disable_batch_input_file_rate_limiting";
const SKIP_PROVIDERS_PATH: &str =
    "general_settings.skip_batch_input_file_rate_limiting_for_providers";
const SKIP_MODELS_PATH: &str = "general_settings.skip_batch_input_file_rate_limiting_for_models";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(rename = "disable_batch_input_file_rate_limiting")]
    disable: Option<JsonValue>,
    #[serde(rename = "skip_batch_input_file_rate_limiting_for_providers")]
    skip_providers: Option<JsonValue>,
    #[serde(rename = "skip_batch_input_file_rate_limiting_for_models")]
    skip_models: Option<JsonValue>,
}

// Every key defaults to absent; a skip list of no entries is a list, not an unset key
#[derive(Debug, Clone, Default, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    #[serde(rename = "disable_batch_input_file_rate_limiting")]
    pub disable_rate_limiting: Option<bool>,
    #[serde(rename = "skip_batch_input_file_rate_limiting_for_providers")]
    pub skip_providers: Option<Vec<String>>,
    #[serde(rename = "skip_batch_input_file_rate_limiting_for_models")]
    pub skip_models: Option<Vec<String>>,
}

// A wrong-typed key names its path; the YAML reader coerces quoted scalars, so the type is checked here
fn names<L>(
    value: Option<JsonValue>,
    path: &str,
    lookup: &L,
) -> Result<Option<Vec<String>>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(raw) = value.filter(|entry| !matches!(entry, JsonValue::Null)) else {
        return Ok(None);
    };
    let JsonValue::Array(items) = raw else {
        return Err(ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        });
    };
    let mut names = Vec::with_capacity(items.len());
    for (index, item) in items.into_iter().enumerate() {
        let JsonValue::String(name) = item else {
            return Err(ConfigError::InvalidScalar {
                path: format!("{path}.{index}"),
                value: false,
            });
        };
        names.push(resolve_string(name, &format!("{path}.{index}"), lookup)?);
    }
    Ok(Some(names))
}

fn flag<L>(value: Option<JsonValue>, path: &str, lookup: &L) -> Result<Option<bool>, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Bool(flag)) => Ok(Some(flag)),
        Some(JsonValue::String(reference)) if reference.starts_with("os.environ/") => {
            let text = resolve_string(reference, path, lookup)?;
            Ok(betterlite_env::flag(path, &text)?)
        }
        Some(
            JsonValue::String(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        ) => Err(ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        }),
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            disable_rate_limiting: flag(self.general_settings.disable, DISABLE_PATH, lookup)?,
            skip_providers: names(
                self.general_settings.skip_providers,
                SKIP_PROVIDERS_PATH,
                lookup,
            )?,
            skip_models: names(self.general_settings.skip_models, SKIP_MODELS_PATH, lookup)?,
        })
    }
}
