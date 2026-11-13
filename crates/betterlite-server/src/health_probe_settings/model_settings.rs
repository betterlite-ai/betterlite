use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

// The health_check_* knobs a deployment carries in model_info, read once per probe.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelHealthSettings {
    pub mode: Option<String>,
    pub max_tokens: Option<i64>,
    pub max_tokens_reasoning: Option<i64>,
    pub max_tokens_non_reasoning: Option<i64>,
    pub supports_max_tokens: Option<bool>,
    pub reasoning_effort: Option<Json>,
    pub model: Option<String>,
    pub voice: Option<String>,
    pub params: Option<JsonObject>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HealthSettingsError {
    #[error("model_info.{field} must be {expected}")]
    Invalid {
        field: &'static str,
        expected: &'static str,
    },
}

fn declared_mode(model_info: &JsonObject) -> Option<String> {
    match model_info.get("mode") {
        Some(Json::Text(mode)) => Some(mode.clone()),
        _ => None,
    }
}

fn integer_field(
    model_info: &JsonObject,
    field: &'static str,
) -> Result<Option<i64>, HealthSettingsError> {
    match model_info.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => {
            number
                .as_i64()
                .map(Some)
                .ok_or(HealthSettingsError::Invalid {
                    field,
                    expected: "an integer",
                })
        }
        // int() reads a numeric string, so a quoted limit in YAML still counts
        Some(Json::Text(text)) => {
            text.trim()
                .parse::<i64>()
                .map(Some)
                .map_err(|_source| HealthSettingsError::Invalid {
                    field,
                    expected: "an integer",
                })
        }
        Some(_) => Err(HealthSettingsError::Invalid {
            field,
            expected: "an integer",
        }),
    }
}

fn bool_field(
    model_info: &JsonObject,
    field: &'static str,
) -> Result<Option<bool>, HealthSettingsError> {
    match model_info.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(HealthSettingsError::Invalid {
            field,
            expected: "a boolean",
        }),
    }
}

fn text_field(
    model_info: &JsonObject,
    field: &'static str,
) -> Result<Option<String>, HealthSettingsError> {
    match model_info.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(HealthSettingsError::Invalid {
            field,
            expected: "a string",
        }),
    }
}

fn value_field(model_info: &JsonObject, field: &str) -> Option<Json> {
    match model_info.get(field) {
        None | Some(Json::Null) => None,
        Some(value) => Some(value.clone()),
    }
}

fn params_field(model_info: &JsonObject) -> Option<JsonObject> {
    match model_info.get("health_check_params") {
        None | Some(Json::Null) => None,
        Some(Json::Object(params)) => Some(params.clone()),
        Some(_) => {
            tracing::warn!("model_info.health_check_params is not a dict; ignoring it");
            None
        }
    }
}

impl ModelHealthSettings {
    pub fn parse(model_info: &JsonObject) -> Result<Self, HealthSettingsError> {
        Ok(Self {
            mode: declared_mode(model_info),
            max_tokens: integer_field(model_info, "health_check_max_tokens")?,
            max_tokens_reasoning: integer_field(model_info, "health_check_max_tokens_reasoning")?,
            max_tokens_non_reasoning: integer_field(
                model_info,
                "health_check_max_tokens_non_reasoning",
            )?,
            supports_max_tokens: bool_field(model_info, "health_check_supports_max_tokens")?,
            reasoning_effort: value_field(model_info, "health_check_reasoning_effort"),
            model: text_field(model_info, "health_check_model")?,
            voice: text_field(model_info, "health_check_voice")?,
            params: params_field(model_info),
        })
    }
}
