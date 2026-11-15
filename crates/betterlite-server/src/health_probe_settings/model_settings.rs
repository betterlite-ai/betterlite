use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

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

trait FieldValue: Sized {
    const EXPECTED: &'static str;

    fn from_json(value: &Json) -> Option<Self>;
}

impl FieldValue for i64 {
    const EXPECTED: &'static str = "an integer";

    fn from_json(value: &Json) -> Option<Self> {
        match value {
            Json::Number(number) => number.as_i64(),
            // int() reads a numeric string, so a quoted limit in YAML still counts
            Json::Text(text) => text.trim().parse().ok(),
            Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
        }
    }
}

impl FieldValue for bool {
    const EXPECTED: &'static str = "a boolean";

    fn from_json(value: &Json) -> Option<Self> {
        match value {
            Json::Bool(flag) => Some(*flag),
            Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
        }
    }
}

impl FieldValue for String {
    const EXPECTED: &'static str = "a string";

    fn from_json(value: &Json) -> Option<Self> {
        match value {
            Json::Text(text) => Some(text.clone()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        }
    }
}

fn field_value<T>(
    model_info: &JsonObject,
    field: &'static str,
) -> Result<Option<T>, HealthSettingsError>
where
    T: FieldValue,
{
    match model_info.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(value) => T::from_json(value)
            .map(Some)
            .ok_or(HealthSettingsError::Invalid {
                field,
                expected: T::EXPECTED,
            }),
    }
}

fn declared_mode(model_info: &JsonObject) -> Option<String> {
    match model_info.get("mode") {
        Some(Json::Text(mode)) => Some(mode.clone()),
        _ => None,
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
            max_tokens: field_value(model_info, "health_check_max_tokens")?,
            max_tokens_reasoning: field_value(model_info, "health_check_max_tokens_reasoning")?,
            max_tokens_non_reasoning: field_value(
                model_info,
                "health_check_max_tokens_non_reasoning",
            )?,
            supports_max_tokens: field_value(model_info, "health_check_supports_max_tokens")?,
            reasoning_effort: value_field(model_info, "health_check_reasoning_effort"),
            model: field_value(model_info, "health_check_model")?,
            voice: field_value(model_info, "health_check_voice")?,
            params: params_field(model_info),
        })
    }
}
