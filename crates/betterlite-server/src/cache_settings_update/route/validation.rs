use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;
use serde::Serialize;

const FIELD_REQUIRED: &str = "Field required";
const JSON_DECODE_ERROR: &str = "JSON decode error";
const DICT_TYPE_MESSAGE: &str = "Input should be a valid dictionary";
const MODEL_ATTRIBUTES_MESSAGE: &str =
    "Input should be a valid dictionary or object to extract fields from";

#[derive(Serialize)]
struct ValidationBody {
    detail: Vec<ValidationIssue>,
}

#[derive(Serialize)]
struct ValidationIssue {
    #[serde(rename = "type")]
    kind: &'static str,
    loc: &'static [&'static str],
    msg: &'static str,
}

// FastAPI answers a bad body with one issue; a null body counts as absent and a non-object body never reaches the model.
pub(super) enum BodyError {
    MissingBody,
    BodyNotAnObject,
    MissingCacheSettings,
    CacheSettingsNotAnObject,
    InvalidJson,
}

// The decoded "cache_settings" member: LiteLLM's request model requires an object and drops the rest of the body.
pub(super) fn decode_request(body: &[u8]) -> Result<JsonMap, BodyError> {
    if body.is_empty() {
        return Err(BodyError::MissingBody);
    }
    match serde_json::from_slice(body).map_err(|_error| BodyError::InvalidJson)? {
        JsonValue::Object(mut fields) => match fields.remove("cache_settings") {
            Some(JsonValue::Object(cache_settings)) => Ok(cache_settings),
            Some(
                JsonValue::Null
                | JsonValue::Bool(_)
                | JsonValue::Number(_)
                | JsonValue::String(_)
                | JsonValue::Array(_),
            ) => Err(BodyError::CacheSettingsNotAnObject),
            None => Err(BodyError::MissingCacheSettings),
        },
        JsonValue::Null => Err(BodyError::MissingBody),
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) | JsonValue::Array(_) => {
            Err(BodyError::BodyNotAnObject)
        }
    }
}

impl BodyError {
    const fn issue(&self) -> ValidationIssue {
        match self {
            Self::MissingBody => ValidationIssue {
                kind: "missing",
                loc: &["body"],
                msg: FIELD_REQUIRED,
            },
            Self::BodyNotAnObject => ValidationIssue {
                kind: "model_attributes_type",
                loc: &["body"],
                msg: MODEL_ATTRIBUTES_MESSAGE,
            },
            Self::MissingCacheSettings => ValidationIssue {
                kind: "missing",
                loc: &["body", "cache_settings"],
                msg: FIELD_REQUIRED,
            },
            Self::CacheSettingsNotAnObject => ValidationIssue {
                kind: "dict_type",
                loc: &["body", "cache_settings"],
                msg: DICT_TYPE_MESSAGE,
            },
            Self::InvalidJson => ValidationIssue {
                kind: "json_invalid",
                loc: &["body"],
                msg: JSON_DECODE_ERROR,
            },
        }
    }
}

impl IntoResponse for BodyError {
    fn into_response(self) -> Response {
        let body = ValidationBody {
            detail: vec![self.issue()],
        };
        (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
    }
}
