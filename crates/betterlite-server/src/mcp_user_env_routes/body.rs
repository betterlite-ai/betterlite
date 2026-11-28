use std::collections::BTreeMap;
use std::str;

use axum::Json;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;
use thiserror::Error;

use super::types::MCPUserEnvVarsRequest;

const REQUIRED: &str = "Field required";
const JSON_DECODE_ERROR: &str = "JSON decode error";
const ATTRIBUTES_TYPE: &str = "Input should be a valid dictionary or object to extract fields from";
const DICT_TYPE: &str = "Input should be a valid dictionary";
const STRING_TYPE: &str = "Input should be a valid string";
const UNREADABLE_BODY: &str = "There was an error parsing the body";

// FastAPI's items carry type, loc and msg; its input and ctx would echo a submitted secret (deviations)
#[derive(Debug, Eq, PartialEq, Serialize)]
pub(crate) struct Issue {
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
    pub(crate) loc: Vec<String>,
    pub(crate) msg: &'static str,
}

#[derive(Debug, Error)]
pub(crate) enum BodyRefusal {
    #[error("the request body does not match the declared types")]
    Validation(Vec<Issue>),
    #[error("the request body could not be read")]
    Unreadable,
}

#[derive(Serialize)]
struct IssuesBody {
    detail: Vec<Issue>,
}

#[derive(Serialize)]
struct TextBody {
    detail: &'static str,
}

impl BodyRefusal {
    pub(crate) const fn class_name(&self) -> &'static str {
        match self {
            Self::Validation(_) => "RequestValidationError",
            Self::Unreadable => "HTTPException",
        }
    }

    pub(crate) const fn status(&self) -> StatusCode {
        match self {
            Self::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Unreadable => StatusCode::BAD_REQUEST,
        }
    }
}

impl IntoResponse for BodyRefusal {
    fn into_response(self) -> Response {
        match self {
            Self::Validation(issues) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(IssuesBody { detail: issues }),
            )
                .into_response(),
            Self::Unreadable => (
                StatusCode::BAD_REQUEST,
                Json(TextBody {
                    detail: UNREADABLE_BODY,
                }),
            )
                .into_response(),
        }
    }
}

// FastAPI reads the body as JSON only when no content type or an application/json one is sent
pub(crate) fn decode_user_env_vars_request(
    content_type: Option<&HeaderValue>,
    body: &[u8],
) -> Result<MCPUserEnvVarsRequest, BodyRefusal> {
    if body.is_empty() {
        return Err(BodyRefusal::Validation(vec![issue(
            "missing",
            &["body"],
            REQUIRED,
        )]));
    }
    if !is_json_media_type(content_type) {
        return Err(BodyRefusal::Validation(vec![issue(
            "model_attributes_type",
            &["body"],
            ATTRIBUTES_TYPE,
        )]));
    }
    let Ok(text) = str::from_utf8(body) else {
        return Err(BodyRefusal::Unreadable);
    };
    let Ok(parsed) = serde_json::from_str::<Json>(text) else {
        return Err(BodyRefusal::Validation(vec![issue(
            "json_invalid",
            &["body"],
            JSON_DECODE_ERROR,
        )]));
    };
    match parsed {
        Json::Null => Err(BodyRefusal::Validation(vec![issue(
            "missing",
            &["body"],
            REQUIRED,
        )])),
        Json::Object(fields) => values_from_body(fields),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(BodyRefusal::Validation(vec![issue(
                "model_attributes_type",
                &["body"],
                ATTRIBUTES_TYPE,
            )]))
        }
    }
}

fn values_from_body(
    mut fields: IndexMap<String, Json>,
) -> Result<MCPUserEnvVarsRequest, BodyRefusal> {
    let Some(values) = fields.shift_remove("values") else {
        return Err(BodyRefusal::Validation(vec![issue(
            "missing",
            &["body", "values"],
            REQUIRED,
        )]));
    };
    let Json::Object(entries) = values else {
        return Err(BodyRefusal::Validation(vec![issue(
            "dict_type",
            &["body", "values"],
            DICT_TYPE,
        )]));
    };
    let mut accepted = BTreeMap::new();
    let mut issues = Vec::new();
    for (name, value) in entries {
        match value {
            Json::Text(text) => {
                accepted.insert(name, text);
            }
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                issues.push(issue(
                    "string_type",
                    &["body", "values", name.as_str()],
                    STRING_TYPE,
                ));
            }
        }
    }
    if issues.is_empty() {
        Ok(MCPUserEnvVarsRequest { values: accepted })
    } else {
        Err(BodyRefusal::Validation(issues))
    }
}

// FastAPI's test: an absent or empty header parses JSON, and so does an application/*+json subtype
fn is_json_media_type(content_type: Option<&HeaderValue>) -> bool {
    let Some(content_type) = content_type else {
        return true;
    };
    let Ok(value) = content_type.to_str() else {
        return false;
    };
    if value.is_empty() {
        return true;
    }
    let essence = value.split(';').next().unwrap_or_default().trim();
    let Some((main, subtype)) = essence.split_once('/') else {
        return false;
    };
    main.eq_ignore_ascii_case("application")
        && (subtype.eq_ignore_ascii_case("json") || subtype.to_ascii_lowercase().ends_with("+json"))
}

fn issue(kind: &'static str, loc: &[&str], msg: &'static str) -> Issue {
    Issue {
        kind,
        loc: loc.iter().map(|part| String::from(*part)).collect(),
        msg,
    }
}
