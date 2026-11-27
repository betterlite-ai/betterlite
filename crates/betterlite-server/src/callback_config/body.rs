use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_config::sections::settings::JsonValue;
use serde::Serialize;

// FastAPI's RequestValidationError renders one detail element per body parameter, in this shape
#[derive(Serialize)]
struct ValidationIssue {
    #[serde(rename = "type")]
    kind: &'static str,
    loc: Vec<&'static str>,
    msg: &'static str,
    input: JsonValue,
}

#[derive(Serialize)]
struct ValidationBody {
    detail: Vec<ValidationIssue>,
}

pub enum BodyError {
    MissingBody,
    NotAnObject(JsonValue),
    InvalidJson,
    MissingField {
        field: &'static str,
        input: JsonValue,
    },
    NotAString {
        field: &'static str,
        input: JsonValue,
    },
}

impl IntoResponse for BodyError {
    fn into_response(self) -> Response {
        let issue = match self {
            Self::MissingBody => ValidationIssue {
                kind: "missing",
                loc: vec!["body"],
                msg: "Field required",
                input: JsonValue::Null,
            },
            Self::NotAnObject(input) => ValidationIssue {
                kind: "model_attributes_type",
                loc: vec!["body"],
                msg: "Input should be a valid dictionary or object to extract fields from",
                input,
            },
            Self::InvalidJson => ValidationIssue {
                kind: "json_invalid",
                loc: vec!["body"],
                msg: "JSON decode error",
                input: JsonValue::Null,
            },
            Self::MissingField { field, input } => ValidationIssue {
                kind: "missing",
                loc: vec!["body", field],
                msg: "Field required",
                input,
            },
            Self::NotAString { field, input } => ValidationIssue {
                kind: "string_type",
                loc: vec!["body", field],
                msg: "Input should be a valid string",
                input,
            },
        };
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ValidationBody {
                detail: vec![issue],
            }),
        )
            .into_response()
    }
}

pub struct CallbackDelete {
    pub(crate) callback_name: String,
}

// CallbackDelete.callback_name is a required str, so FastAPI answers a missing key and a non-string alike
pub fn callback_delete_request(body: &[u8]) -> Result<CallbackDelete, BodyError> {
    let mut fields = match parse_body(body)? {
        JsonValue::Object(fields) => fields,
        other @ (JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_)) => return Err(BodyError::NotAnObject(other)),
    };
    match fields.remove("callback_name") {
        None => Err(BodyError::MissingField {
            field: "callback_name",
            input: JsonValue::Object(fields),
        }),
        Some(JsonValue::String(callback_name)) => Ok(CallbackDelete { callback_name }),
        Some(other) => Err(BodyError::NotAString {
            field: "callback_name",
            input: other,
        }),
    }
}

// config_yaml_endpoint declares a ConfigYAML body parameter, so a GET without one is a 422 before the mock runs
pub fn config_yaml_request(body: &[u8]) -> Result<(), BodyError> {
    let value = parse_body(body)?;
    if value.is_object() {
        return Ok(());
    }
    Err(BodyError::NotAnObject(value))
}

fn parse_body(body: &[u8]) -> Result<JsonValue, BodyError> {
    if body.is_empty() {
        return Err(BodyError::MissingBody);
    }
    serde_json::from_slice(body).map_err(|_error| BodyError::InvalidJson)
}
