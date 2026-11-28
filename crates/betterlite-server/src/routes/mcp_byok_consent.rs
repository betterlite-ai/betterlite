mod tests;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::Router;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse as _, Response};
use axum::routing::get;
use betterlite_auth::auth_key_lookup::KeyAuthenticator;
use betterlite_core::api::request_form::values::FormParts;
use betterlite_mcp::byok_consent::ConsentError;
use betterlite_mcp::byok_consent::get::{DEFAULT_CODE_CHALLENGE_METHOD, GetQuery, authorize_get};
use betterlite_mcp::byok_consent::post::{Accepted, PostForm, authorize_post};
use betterlite_mcp::byok_consent::session::authenticate_session;
use betterlite_mcp::byok_consent::store::AuthCodeStore;
use betterlite_mcp::session_admission::auth::SessionAuthError;
use chrono::Utc;
use secrecy::SecretString;

use crate::api::request_form::{
    DetailBody, FORM_CONTENT_TYPES, ValidationBody, last_value, read_form, validation_body,
};
use crate::state::ServerState;

pub(crate) const AUTHORIZE_PATH: &str = "/v1/mcp/oauth/authorize";
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
    let cookie = request
        .headers()
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok());
    let user_id = match session_user(&state, cookie).await {
        Ok(user_id) => user_id,
        Err(response) => return *response,
    };
    let form_parts = match read_form(request, FORM_CONTENT_TYPES).await {
        Ok(form_parts) => form_parts,
        Err(error) => {
            let body = DetailBody {
                detail: error.detail(),
            };
            return (StatusCode::BAD_REQUEST, Json(body)).into_response();
        }
    };
    submit_accepted(&state, &user_id, &form_parts)
}

async fn session_user(
    state: &ConsentState,
    cookie_header: Option<&str>,
) -> Result<String, Box<Response>> {
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
            Err(Box::new(
                (error.status(), Json(error.body())).into_response(),
            ))
        }
        Err(error @ SessionAuthError::Construct(_)) => {
            tracing::error!("mcp byok consent: the session principal did not construct: {error}");
            Err(Box::new(
                (StatusCode::INTERNAL_SERVER_ERROR, INTERNAL_ERROR).into_response(),
            ))
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
