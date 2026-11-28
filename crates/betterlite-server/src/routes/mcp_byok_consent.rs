mod tests;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::Router;
use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse as _, Response};
use axum::routing::get;
use betterlite_auth::auth_key_lookup::KeyAuthenticator;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_form::values::{FormParts, PartValue};
use betterlite_mcp::byok_consent::ConsentError;
use betterlite_mcp::byok_consent::get::{DEFAULT_CODE_CHALLENGE_METHOD, GetQuery, authorize_get};
use betterlite_mcp::byok_consent::post::{Accepted, PostForm, authorize_post};
use betterlite_mcp::byok_consent::session::authenticate_session;
use betterlite_mcp::byok_consent::store::AuthCodeStore;
use betterlite_mcp::session_admission::auth::SessionAuthError;
use chrono::Utc;
use secrecy::SecretString;
use serde::Serialize;

use crate::api::request_form::{RequestFormError, decode_request_form};
use crate::state::ServerState;

pub(crate) const AUTHORIZE_PATH: &str = "/v1/mcp/oauth/authorize";
const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";
// LiteLLM caps one form field at 1 MiB but leaves the total unbounded; one MiB for the whole body closes that.
const BODY_LIMIT_BYTES: usize = 0x0010_0000;
const MAX_FORM_FIELDS: usize = 1000;
const FORM_LIMITS: ApiLimits = ApiLimits {
    max_json_bytes: BODY_LIMIT_BYTES,
    max_event_bytes: BODY_LIMIT_BYTES,
    max_items: MAX_FORM_FIELDS,
};
// FastAPI's declaration order decides the order of the 422 entries.
const REQUIRED_FIELDS: [&str; 3] = ["redirect_uri", "code_challenge", "api_key"];
const INTERNAL_ERROR: &str = "Internal Server Error";

struct ConsentState {
    codes: AuthCodeStore,
    master_key: Option<SecretString>,
    authenticator: Option<Arc<dyn KeyAuthenticator>>,
}

#[cfg_attr(test, mutants::skip)]
pub(crate) fn routes(state: &Arc<ServerState>) -> Router {
    router_with(Arc::new(ConsentState {
        codes: AuthCodeStore::new(),
        master_key: state.config.general_settings.master_key.clone(),
        authenticator: state.auth_key_lookup.authenticator.clone(),
    }))
}

fn router_with(consent: Arc<ConsentState>) -> Router {
    Router::new()
        .route(AUTHORIZE_PATH, get(authorize_page).post(submit_form))
        .with_state(consent)
}

// The GET is unauthenticated: it only renders the form, and every state change happens on the POST.
async fn authorize_page(request: Request) -> Response {
    let query = query_from(request.uri().query());
    // B8624 wires the MCP registry into the server state; until then every page shows the fallback.
    match authorize_get(&query, None) {
        Ok(html) => Html(html).into_response(),
        Err(error) => (error.status(), Json(error.body())).into_response(),
    }
}

async fn submit_form(State(state): State<Arc<ConsentState>>, request: Request) -> Response {
    let cookie_header = cookie_header(request.headers());
    let user_id = match session_user(&state, cookie_header).await {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    let form_parts = match read_form(request).await {
        Ok(form_parts) => form_parts,
        Err(error) => {
            let body = DetailBody {
                detail: error.to_string(),
            };
            return (StatusCode::BAD_REQUEST, Json(body)).into_response();
        }
    };
    submit_accepted(&state, &user_id, &form_parts)
}

async fn session_user(
    state: &ConsentState,
    cookie_header: Option<&str>,
) -> Result<String, Response> {
    let authenticated = authenticate_session(
        cookie_header,
        state.master_key.as_ref(),
        state.authenticator.as_deref(),
        Utc::now(),
    )
    .await;
    match authenticated {
        Ok(user_id) => Ok(user_id),
        Err(SessionAuthError::LoginRequired) => {
            let error = ConsentError::LoginRequired;
            Err((error.status(), Json(error.body())).into_response())
        }
        Err(error @ SessionAuthError::Construct(_)) => {
            tracing::error!("mcp byok consent: the session principal did not construct: {error}");
            Err((StatusCode::INTERNAL_SERVER_ERROR, INTERNAL_ERROR).into_response())
        }
    }
}

fn submit_accepted(state: &ConsentState, user_id: &str, form_parts: &FormParts) -> Response {
    let form = match post_form(form_parts) {
        Ok(form) => form,
        Err(rejection) => {
            return (StatusCode::UNPROCESSABLE_ENTITY, Json(rejection)).into_response();
        }
    };
    let Ok(clock) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, INTERNAL_ERROR).into_response();
    };
    match authorize_post(&form, user_id, &state.codes, clock.as_secs_f64()) {
        Ok(Accepted { location, .. }) => {
            (StatusCode::FOUND, [(header::LOCATION, location)]).into_response()
        }
        Err(error) => (error.status(), Json(error.body())).into_response(),
    }
}

fn query_from(query: Option<&str>) -> GetQuery {
    let mut parsed = GetQuery::default();
    for (name, value) in form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
        parsed.set(name.as_ref(), value.into_owned());
    }
    parsed
}

fn cookie_header(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
}

async fn read_form(request: Request) -> Result<FormParts, FormError> {
    let (parts, body) = request.into_parts();
    let Some(content_type) = form_content_type(&parts.headers) else {
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

#[derive(Debug, thiserror::Error)]
enum FormError {
    #[error("There was an error parsing the body")]
    UnreadableBody,
    #[error("Too many fields. Maximum number of fields is 1000.")]
    TooManyFields,
}

fn form_content_type(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::CONTENT_TYPE)?;
    let text = value.to_str().ok()?;
    let media_type = text.split(';').next().unwrap_or_default().trim();
    media_type
        .eq_ignore_ascii_case(FORM_CONTENT_TYPE)
        .then_some(text)
}

fn post_form(parts: &FormParts) -> Result<PostForm, ValidationBody> {
    let missing: Vec<&'static str> = REQUIRED_FIELDS
        .iter()
        .copied()
        .filter(|field| last_value(parts, field).is_none())
        .collect();
    if !missing.is_empty() {
        return Err(validation_body(&missing));
    }
    let (Some(redirect_uri), Some(code_challenge), Some(api_key)) = (
        last_value(parts, "redirect_uri"),
        last_value(parts, "code_challenge"),
        last_value(parts, "api_key"),
    ) else {
        return Err(validation_body(&REQUIRED_FIELDS));
    };
    Ok(PostForm {
        client_id: last_value(parts, "client_id")
            .unwrap_or_default()
            .to_owned(),
        redirect_uri: redirect_uri.to_owned(),
        code_challenge: code_challenge.to_owned(),
        code_challenge_method: last_value(parts, "code_challenge_method")
            .unwrap_or(DEFAULT_CODE_CHALLENGE_METHOD)
            .to_owned(),
        state: last_value(parts, "state").unwrap_or_default().to_owned(),
        server_id: last_value(parts, "server_id")
            .unwrap_or_default()
            .to_owned(),
        api_key: SecretString::from(api_key.to_owned()),
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
    detail: String,
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
