mod tests;

use crate::loader::{ConfigError, EnvironmentLookup, parse_view};
use crate::resolve::resolve_string;
use crate::sections::settings::JsonValue;
use serde::Deserialize;

const CONTROL_PLANE_URL_PATH: &str = "general_settings.control_plane_url";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Raw {
    general_settings: Option<GeneralSettings>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct GeneralSettings {
    control_plane_url: Option<JsonValue>,
}

// LiteLLM reads general_settings["control_plane_url"] as a str or None; get_secret resolves an os.environ/ reference first
pub fn load_control_plane_url<Lookup>(
    source: &str,
    lookup: &Lookup,
) -> Result<Option<String>, ConfigError>
where
    Lookup: EnvironmentLookup,
{
    let (raw, _ignored) = parse_view::<Raw>(source)?;
    let Some(general) = raw.general_settings else {
        return Ok(None);
    };
    let Some(configured) = general.control_plane_url else {
        return Ok(None);
    };
    match configured {
        JsonValue::Null => Ok(None),
        JsonValue::String(text) => resolve_string(text, CONTROL_PLANE_URL_PATH, lookup).map(Some),
        value @ (JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_)) => Err(invalid_scalar(&value)),
    }
}

fn invalid_scalar(value: &JsonValue) -> ConfigError {
    ConfigError::InvalidScalar {
        path: CONTROL_PLANE_URL_PATH.to_owned(),
        value: matches!(value, JsonValue::Bool(true)),
    }
}
