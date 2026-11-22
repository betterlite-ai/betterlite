mod tests;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_form::values::{FormParts, PartValue};
use betterlite_mcp::byok_token_exchange::exchange::exchange_authorization_code;
use betterlite_mcp::byok_token_exchange::ports::{AuthCodeLookup, ByokCredentialSink};
use betterlite_mcp::byok_token_exchange::{TokenError, TokenErrorResponse, TokenForm};
use betterlite_mcp::oauth_resource::challenge::TOKEN_NO_CACHE_HEADERS;
use serde::Serialize;
use thiserror::Error;

use crate::api::request_form::{RequestFormError, decode_request_form};
use crate::routes::mcp_byok_oauth::{ByokOauthRoutes, RequestBaseUrl};

const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";
// LiteLLM caps one form field at 1 MiB but leaves the total unbounded; one MiB for the whole body closes that.
const BODY_LIMIT_BYTES: usize = 0x0010_0000;
const MAX_FORM_FIELDS: usize = 1000;
// The shared decoder caps ampersand-separated parts, and Starlette counts only fields, so an empty segment spends one of the 1000.
const FORM_LIMITS: ApiLimits = ApiLimits {
    max_json_bytes: BODY_LIMIT_BYTES,
    max_event_bytes: BODY_LIMIT_BYTES,
    max_items: MAX_FORM_FIELDS,
};
const REQUIRED_FIELDS: [&str; 3] = ["grant_type", "code", "code_verifier"];
const UNREADABLE_BODY: &str = "There was an error parsing the body";
const TOO_MANY_FIELDS: &str = "Too many fields. Maximum number of fields is 1000.";

pub(crate) async fn token_endpoint<C, S, B>(
    State(state): State<Arc<ByokOauthRoutes<C, S, B>>>,
    request: Request,
) -> Response
where
    C: AuthCodeLookup + 'static,
    S: ByokCredentialSink + 'static,
    B: RequestBaseUrl + 'static,
{
    let (parts, body) = request.into_parts();
    let form_parts = match read_form(&parts.headers, body).await {
        Ok(form_parts) => form_parts,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(DetailBody {
                    detail: error.detail(),
                }),
            )
                .into_response();
        }
    };
    let form = match required_form(&form_parts) {
        Ok(form) => form,
        Err(rejection) => {
            return (StatusCode::UNPROCESSABLE_ENTITY, Json(rejection)).into_response();
        }
    };
    let Ok(clock) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        let TokenErrorResponse { status, body } = TokenError::ServerError.response();
        return (status, TOKEN_NO_CACHE_HEADERS, Json(body)).into_response();
    };
    let result = exchange_authorization_code(
        &form,
        &state.codes,
        state.credentials.as_ref(),
        state.master_key.as_ref(),
        clock.as_secs_f64(),
    )
    .await;
    match result {
        Ok(grant) => (StatusCode::OK, TOKEN_NO_CACHE_HEADERS, Json(grant)).into_response(),
        Err(error) => {
            let TokenErrorResponse { status, body } = error.response();
            (status, TOKEN_NO_CACHE_HEADERS, Json(body)).into_response()
        }
    }
}

async fn read_form(headers: &HeaderMap, body: Body) -> Result<FormParts, FormError> {
    let Some(content_type) = form_content_type(headers) else {
        return Ok(FormParts::default());
    };
    let Ok(bytes) = to_bytes(body, BODY_LIMIT_BYTES).await else {
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
enum FormError {
    #[error("There was an error parsing the body")]
    UnreadableBody,
    #[error("Too many fields. Maximum number of fields is 1000.")]
    TooManyFields,
}

impl FormError {
    const fn detail(self) -> &'static str {
        match self {
            Self::UnreadableBody => UNREADABLE_BODY,
            Self::TooManyFields => TOO_MANY_FIELDS,
        }
    }
}

fn form_content_type(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::CONTENT_TYPE)?;
    let text = value.to_str().ok()?;
    let media_type = text.split(';').next().unwrap_or_default().trim();
    media_type
        .eq_ignore_ascii_case(FORM_CONTENT_TYPE)
        .then_some(text)
}

fn required_form(parts: &FormParts) -> Result<TokenForm, ValidationBody> {
    let missing: Vec<&'static str> = REQUIRED_FIELDS
        .iter()
        .copied()
        .filter(|field| last_value(parts, field).is_none_or(str::is_empty))
        .collect();
    if !missing.is_empty() {
        return Err(validation_body(&missing));
    }
    let (Some(grant_type), Some(code), Some(code_verifier)) = (
        last_value(parts, "grant_type"),
        last_value(parts, "code"),
        last_value(parts, "code_verifier"),
    ) else {
        return Err(validation_body(&REQUIRED_FIELDS));
    };
    Ok(TokenForm {
        grant_type: grant_type.to_owned(),
        code: code.to_owned(),
        redirect_uri: last_value(parts, "redirect_uri")
            .unwrap_or_default()
            .to_owned(),
        code_verifier: code_verifier.to_owned(),
        client_id: last_value(parts, "client_id")
            .unwrap_or_default()
            .to_owned(),
    })
}

fn last_value<'parts>(parts: &'parts FormParts, field: &str) -> Option<&'parts str> {
    parts.items.iter().rev().find_map(|part| match &part.value {
        PartValue::Text(value) if part.name == field => Some(value.as_str()),
        PartValue::Text(_) | PartValue::Upload(_) => None,
    })
}

fn validation_body(missing: &[&'static str]) -> ValidationBody {
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
struct DetailBody {
    detail: &'static str,
}

#[derive(Debug, Serialize)]
struct ValidationBody {
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
