use axum::body::to_bytes;
use axum::extract::Request;
use axum::http::{HeaderMap, header};
use betterlite_core::api::api_boundaries::{ApiBoundariesInput, ApiLimits, JsonBoundary};
use betterlite_core::api::request_body::{
    DecodedJson, MediaTypeKind, RequestBodyError, classify_media_type,
};
use betterlite_core::api::request_form::values::{FormFields, FormParts, PartValue, get_form_data};
use serde::Serialize;
use std::str;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RequestFormError {
    #[error("request form body is too large")]
    BodyTooLarge,
    #[error("request form has too many parts")]
    TooManyParts,
    #[error("invalid request form content type")]
    ContentType,
    #[error("invalid request form body")]
    InvalidBody,
    #[error("request form field name is missing")]
    MissingName,
    #[error("Unsupported content type: {0}")]
    UnsupportedContentType(String),
    #[error(transparent)]
    RequestBody(#[from] RequestBodyError),
}

#[derive(Debug, PartialEq, Eq)]
pub enum RequestFormBody {
    Empty,
    Json(DecodedJson),
    Form(FormFields),
}

pub fn get_request_body(
    method: &str,
    content_type: &str,
    body: &[u8],
    limits: &ApiLimits,
) -> Result<RequestFormBody, RequestFormError> {
    if method != "POST" {
        return Ok(RequestFormBody::Empty);
    }
    match classify_media_type(content_type) {
        MediaTypeKind::Json => Ok(RequestFormBody::Json(
            super::request_body::decode_request_body(body, limits)?,
        )),
        MediaTypeKind::Form => {
            let parts = decode_request_form(body, content_type.as_bytes(), limits)?;
            Ok(RequestFormBody::Form(
                get_form_data(&parts).map_err(|_error| RequestFormError::InvalidBody)?,
            ))
        }
        MediaTypeKind::Binary | MediaTypeKind::Other => Err(
            RequestFormError::UnsupportedContentType(content_type.to_owned()),
        ),
    }
}

// decode_request_form: URL encoded and multipart bytes decode into ordered parts before any field policy runs
pub fn decode_request_form(
    bytes: &[u8],
    content_type: &[u8],
    limits: &ApiLimits,
) -> Result<FormParts, RequestFormError> {
    limits
        .check(ApiBoundariesInput::Json(JsonBoundary {
            known_bytes: bytes.len(),
        }))
        .map_err(|_error| RequestFormError::BodyTooLarge)?;
    limits
        .check(ApiBoundariesInput::Json(JsonBoundary {
            known_bytes: content_type.len(),
        }))
        .map_err(|_error| RequestFormError::BodyTooLarge)?;
    let header_text =
        str::from_utf8(content_type).map_err(|_error| RequestFormError::ContentType)?;
    let media_type = header_text.split(';').next().unwrap_or(header_text);
    match media_type.trim().to_ascii_lowercase().as_str() {
        "application/x-www-form-urlencoded" => decode::urlencoded(bytes, limits),
        "multipart/form-data" => decode::multipart(bytes, header_text, limits),
        _other => Err(RequestFormError::ContentType),
    }
}

// FastAPI's Form(...) reads a body of either form encoding; the routes differ on which ones they declare.
pub(crate) const FORM_CONTENT_TYPES: &[&str] =
    &["application/x-www-form-urlencoded", "multipart/form-data"];
pub(crate) const URLENCODED_FORM_CONTENT_TYPES: &[&str] = &["application/x-www-form-urlencoded"];
// LiteLLM caps one form field at 1 MiB but leaves the total unbounded; one MiB for the whole body closes that.
const FORM_BODY_LIMIT_BYTES: usize = 0x0010_0000;
pub(crate) const MAX_FORM_FIELDS: usize = 1000;
// The shared decoder caps ampersand-separated parts, and Starlette counts only fields, so an empty segment spends one of the 1000.
const FORM_LIMITS: ApiLimits = ApiLimits {
    max_json_bytes: FORM_BODY_LIMIT_BYTES,
    max_event_bytes: FORM_BODY_LIMIT_BYTES,
    max_items: MAX_FORM_FIELDS,
};
const UNREADABLE_BODY: &str = "There was an error parsing the body";
const TOO_MANY_FIELDS: &str = "Too many fields. Maximum number of fields is 1000.";

pub(crate) async fn read_form(
    request: Request,
    accepted_content_types: &[&str],
) -> Result<FormParts, FormError> {
    let (parts, body) = request.into_parts();
    let Some(content_type) = form_content_type(&parts.headers, accepted_content_types) else {
        return Ok(FormParts::default());
    };
    let Ok(bytes) = to_bytes(body, FORM_BODY_LIMIT_BYTES).await else {
        return Err(FormError::UnreadableBody);
    };
    match decode_request_form(&bytes, content_type.as_bytes(), &FORM_LIMITS) {
        Ok(form_parts) => Ok(form_parts),
        Err(RequestFormError::TooManyParts) => Err(FormError::TooManyFields),
        Err(
            RequestFormError::BodyTooLarge
            | RequestFormError::ContentType
            | RequestFormError::InvalidBody
            | RequestFormError::MissingName
            | RequestFormError::UnsupportedContentType(_)
            | RequestFormError::RequestBody(_),
        ) => Err(FormError::UnreadableBody),
    }
}

#[derive(Debug, Error)]
pub(crate) enum FormError {
    #[error("There was an error parsing the body")]
    UnreadableBody,
    #[error("Too many fields. Maximum number of fields is 1000.")]
    TooManyFields,
}

impl FormError {
    pub(crate) const fn detail(self) -> &'static str {
        match self {
            Self::UnreadableBody => UNREADABLE_BODY,
            Self::TooManyFields => TOO_MANY_FIELDS,
        }
    }
}

fn form_content_type<'headers>(
    headers: &'headers HeaderMap,
    accepted_content_types: &[&str],
) -> Option<&'headers str> {
    let value = headers.get(header::CONTENT_TYPE)?;
    let text = value.to_str().ok()?;
    let media_type = text.split(';').next().unwrap_or_default().trim();
    accepted_content_types
        .iter()
        .copied()
        .any(|accepted| media_type.eq_ignore_ascii_case(accepted))
        .then_some(text)
}

pub(crate) fn last_value<'parts>(parts: &'parts FormParts, field: &str) -> Option<&'parts str> {
    parts.items.iter().rev().find_map(|part| match &part.value {
        PartValue::Text(value) if part.name == field => Some(value.as_str()),
        PartValue::Text(_) | PartValue::Upload(_) => None,
    })
}

pub(crate) fn validation_body(missing: &[&'static str]) -> ValidationBody {
    ValidationBody {
        detail: missing
            .iter()
            .map(|field| ValidationDetail {
                kind: "missing",
                loc: ["body", *field],
                msg: "Field required",
                input: NullInput,
            })
            .collect(),
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct DetailBody {
    pub(crate) detail: &'static str,
}

#[derive(Debug, Serialize)]
pub(crate) struct ValidationBody {
    detail: Vec<ValidationDetail>,
}

// FastAPI 0.162's RequestValidationError for a missing Form(...) field.
#[derive(Debug, Serialize)]
struct ValidationDetail {
    #[serde(rename = "type")]
    kind: &'static str,
    loc: [&'static str; 2],
    msg: &'static str,
    input: NullInput,
}

#[derive(Debug, Serialize)]
struct NullInput;

mod decode;
mod tests;
