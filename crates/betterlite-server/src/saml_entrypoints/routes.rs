use std::collections::BTreeMap;
use std::net::SocketAddr;

use axum::Router;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use betterlite_auth::saml_provider::provider::{LoginRedirect, ProviderUrls};
use betterlite_auth::trusted_network::request::{NetworkRequest, is_request_https};
use url::form_urlencoded;

use super::error::{internal_error, provider_error_response};
use super::state::SamlEntrypointsState;
use crate::admin_setup::admin_ui_disabled;
use crate::custom_url::CustomUrls;
use crate::docs_routes::redirect_location;
use crate::middleware::request_root_paths::{get_request_root_path, request_base_url};

pub const SAML_LOGIN_PATH: &str = "/sso/saml/login";
pub const SAML_METADATA_PATH: &str = "/sso/saml/metadata";
const SAML_CALLBACK_ROUTE: &str = "sso/saml/callback";
const SAML_METADATA_ROUTE: &str = "sso/saml/metadata";
const AUTHN_COOKIE: &str = "litellm_saml_authn";
const AUTHN_COOKIE_TTL_SECONDS: i64 = 600;

pub fn routes(state: SamlEntrypointsState) -> Router {
    Router::new()
        .route(SAML_LOGIN_PATH, get(login))
        .route(SAML_METADATA_PATH, get(metadata))
        .with_state(state)
}

async fn login(State(state): State<SamlEntrypointsState>, request: Request) -> Response {
    if disable_admin_ui(&state.environment) {
        return admin_ui_disabled();
    }
    let (urls, base_url) = provider_urls(&state.custom_urls, &request);
    let relay_state = form_urlencoded::parse(request.uri().query().unwrap_or_default().as_bytes())
        .filter(|(key, _value)| key.as_ref() == "return_to")
        .last()
        .map_or_else(
            || default_relay_state(&base_url),
            |(_key, value)| value.into_owned(),
        );
    match state.provider.login(&urls, &relay_state).await {
        Ok(redirect) => login_redirect(&redirect, secure_request(&state, &request)),
        Err(error) => provider_error_response(&error),
    }
}

async fn metadata(State(state): State<SamlEntrypointsState>, request: Request) -> Response {
    let (urls, _base_url) = provider_urls(&state.custom_urls, &request);
    match state.provider.sp_metadata(&urls).await {
        Ok(metadata) => ([(header::CONTENT_TYPE, "application/xml")], metadata).into_response(),
        Err(error) => provider_error_response(&error),
    }
}

// SAMLAuthHandler._base_url, _acs_url and _metadata_url through get_custom_url
fn provider_urls(custom_urls: &CustomUrls, request: &Request) -> (ProviderUrls, String) {
    let root_path = get_request_root_path(request.extensions(), "");
    let resolved = custom_urls.custom_url(&request_base_url(request, &root_path), &root_path);
    let base = if resolved.ends_with('/') {
        resolved
    } else {
        format!("{resolved}/")
    };
    let urls = ProviderUrls {
        metadata_url: format!("{base}{SAML_METADATA_ROUTE}"),
        acs_url: format!("{base}{SAML_CALLBACK_ROUTE}"),
    };
    (urls, base)
}

fn default_relay_state(base_url: &str) -> String {
    let (given, host_part) = base_url.split_once("://").unwrap_or((base_url, ""));
    let host = host_part.split('/').next().unwrap_or_default();
    let scheme = if given == "https" { "https" } else { "http" };
    format!("{scheme}://{host}/{SAML_CALLBACK_ROUTE}")
}

// SAMLAuthHandler._is_https through B743, whose PROXY_BASE_URL and trusted-proxy rules decide the cookie
fn secure_request(state: &SamlEntrypointsState, request: &Request) -> bool {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip().to_string());
    let text = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
    };
    let network = NetworkRequest {
        peer: peer.as_deref(),
        x_forwarded_for: text("x-forwarded-for"),
        x_forwarded_proto: text("x-forwarded-proto"),
        host: text(header::HOST.as_str()),
        scheme: request.uri().scheme_str().unwrap_or_default(),
    };
    is_request_https(&network, &state.trusted_network)
}

// LiteLLM's str_to_bool: only a trimmed "true" disables the UI, and every other value leaves it on
fn disable_admin_ui(environment: &BTreeMap<String, String>) -> bool {
    environment.get("DISABLE_ADMIN_UI").is_some_and(|value| {
        // ast-grep-ignore: no-python-habits - str_to_bool takes only true, while betterlite_env::flag also takes 1, yes and on
        value.trim().eq_ignore_ascii_case("true")
    })
}

// SAMLAuthHandler.build_login_redirect: the 303 Location and the HttpOnly state cookie
fn login_redirect(redirect: &LoginRedirect, secure: bool) -> Response {
    let location = redirect_location(None, &redirect.location);
    let mut response = (StatusCode::SEE_OTHER, [(header::LOCATION, location)]).into_response();
    let Ok(cookie) = HeaderValue::from_str(&authn_cookie(&redirect.request_id, secure)) else {
        return internal_error();
    };
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    response
}

// Starlette's set_cookie order, with SameSite=None only beside Secure
fn authn_cookie(request_id: &str, secure: bool) -> String {
    let same_site = if secure { "none; Secure" } else { "lax" };
    format!(
        "{AUTHN_COOKIE}={request_id}; HttpOnly; Max-Age={AUTHN_COOKIE_TTL_SECONDS}; Path=/; SameSite={same_site}"
    )
}
