mod tests;

use std::str::from_utf8;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use betterlite_mcp::byok_token_exchange::exchange::exchange_authorization_code;
use betterlite_mcp::byok_token_exchange::ports::{AuthCodeLookup, ByokCredentialSink};
use betterlite_mcp::byok_token_exchange::{TokenError, TokenErrorResponse, TokenForm};
use betterlite_mcp::oauth_resource::challenge::TOKEN_NO_CACHE_HEADERS;
use serde::Serialize;
use thiserror::Error;

use crate::routes::mcp_byok_oauth::{ByokOauthRoutes, RequestBaseUrl};

const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";
// LiteLLM caps one form field at 1 MiB but leaves the total unbounded; one MiB for the whole body closes that.
const BODY_LIMIT_BYTES: usize = 1_048_576;
const MAX_FORM_FIELDS: usize = 1000;
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
    let entries = match read_form(&parts.headers, body).await {
        Ok(entries) => entries,
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
    let form = match required_form(&entries) {
        Ok(form) => form,
        Err(body) => return (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response(),
    };
    let Ok(clock) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return token_error_response(TokenError::ServerError);
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
        Err(error) => token_error_response(error),
    }
}

fn token_error_response(error: TokenError) -> Response {
    let TokenErrorResponse { status, body } = error.response();
    (status, TOKEN_NO_CACHE_HEADERS, Json(body)).into_response()
}

async fn read_form(headers: &HeaderMap, body: Body) -> Result<Vec<(String, String)>, FormError> {
    if !is_form_content_type(headers) {
        return Ok(Vec::new());
    }
    let Ok(bytes) = to_bytes(body, BODY_LIMIT_BYTES).await else {
        return Err(FormError::UnreadableBody);
    };
    let entries = parse_form(&bytes);
    if entries.len() > MAX_FORM_FIELDS {
        return Err(FormError::TooManyFields);
    }
    Ok(entries)
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

fn is_form_content_type(headers: &HeaderMap) -> bool {
    let Some(value) = headers.get(header::CONTENT_TYPE) else {
        return false;
    };
    let Ok(text) = value.to_str() else {
        return false;
    };
    text.split(';')
        .next()
        .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case(FORM_CONTENT_TYPE))
}

fn parse_form(bytes: &[u8]) -> Vec<(String, String)> {
    bytes
        .split(|byte| *byte == b'&')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let Some(index) = segment.iter().position(|byte| *byte == b'=') else {
                return (decode_form_component(segment), String::new());
            };
            (
                decode_form_component(segment.get(..index).unwrap_or_default()),
                decode_form_component(segment.get(index.saturating_add(1)..).unwrap_or_default()),
            )
        })
        .collect()
}

// Starlette reads the raw bytes as Latin-1 and then runs unquote_plus, so a raw UTF-8 character becomes mojibake.
fn decode_form_component(raw: &[u8]) -> String {
    let mut bytes: Vec<u8> = Vec::with_capacity(raw.len());
    let mut rest = raw;
    while let Some((first, tail)) = rest.split_first() {
        match *first {
            b'+' => {
                bytes.push(b' ');
                rest = tail;
            }
            b'%' => match hex_pair(tail) {
                Some((high, low)) => {
                    bytes.push(high.saturating_mul(16).saturating_add(low));
                    rest = tail.get(2..).unwrap_or_default();
                }
                None => {
                    bytes.push(b'%');
                    rest = tail;
                }
            },
            byte => {
                let mut buffer = [0_u8; 4];
                bytes.extend_from_slice(char::from(byte).encode_utf8(&mut buffer).as_bytes());
                rest = tail;
            }
        }
    }
    let mut decoded = String::with_capacity(bytes.len());
    push_utf8_replacing(&mut decoded, &bytes);
    decoded
}

fn hex_pair(tail: &[u8]) -> Option<(u8, u8)> {
    let [high, low, ..] = tail else {
        return None;
    };
    let high_value = u8::try_from(char::from(*high).to_digit(16)?).ok()?;
    let low_value = u8::try_from(char::from(*low).to_digit(16)?).ok()?;
    Some((high_value, low_value))
}

fn push_utf8_replacing(output: &mut String, bytes: &[u8]) {
    let mut remaining = bytes;
    while !remaining.is_empty() {
        match from_utf8(remaining) {
            Ok(valid) => {
                output.push_str(valid);
                return;
            }
            Err(error) => {
                let valid_up_to = error.valid_up_to();
                if let Ok(valid) = from_utf8(remaining.get(..valid_up_to).unwrap_or_default()) {
                    output.push_str(valid);
                }
                output.push(char::REPLACEMENT_CHARACTER);
                let consumed = error.error_len().unwrap_or(remaining.len());
                remaining = remaining.get(consumed..).unwrap_or_default();
            }
        }
    }
}

fn required_form(entries: &[(String, String)]) -> Result<TokenForm, ValidationBody> {
    let missing: Vec<&'static str> = REQUIRED_FIELDS
        .iter()
        .copied()
        .filter(|field| last_value(entries, field).is_none_or(str::is_empty))
        .collect();
    if !missing.is_empty() {
        return Err(validation_body(&missing));
    }
    let (Some(grant_type), Some(code), Some(code_verifier)) = (
        last_value(entries, "grant_type"),
        last_value(entries, "code"),
        last_value(entries, "code_verifier"),
    ) else {
        return Err(validation_body(&REQUIRED_FIELDS));
    };
    Ok(TokenForm {
        grant_type: grant_type.to_owned(),
        code: code.to_owned(),
        redirect_uri: last_value(entries, "redirect_uri")
            .unwrap_or_default()
            .to_owned(),
        code_verifier: code_verifier.to_owned(),
        client_id: last_value(entries, "client_id")
            .unwrap_or_default()
            .to_owned(),
    })
}

fn last_value<'entries>(
    entries: &'entries [(String, String)],
    field: &str,
) -> Option<&'entries str> {
    entries
        .iter()
        .rev()
        .find(|(name, _value)| name == field)
        .map(|(_name, value)| value.as_str())
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
