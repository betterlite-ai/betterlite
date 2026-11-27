mod tests;

pub mod native;

use betterlite_db::stored_cipher::DbCipher;

use crate::gateway_authorize::oauth_error;
use crate::gateway_authorize::reply::{FlowCookie, FlowTokenSource, GatewayReply, RequestView};
use crate::gateway_authorize::scope::resolve_scoped_resource_server;
use crate::gateway_authorize::url_parts::{
    append_query_params, cookie_path_and_secure, flow_cookie_name, relative_request_url, urlencode,
};
use crate::gateway_dcr::contracts::{CONNECT_FLOW_TTL_SECONDS, MAX_STATE_LENGTH};
use crate::gateway_dcr::models::ConnectFlow;
use crate::gateway_dcr::oauth_error::OauthError;
use crate::gateway_dcr::oauth_error::OauthErrorCode::{
    InvalidClient, InvalidRequest, ServerError, UnsupportedResponseType,
};
use crate::gateway_dcr::sealed::{open_gateway_dcr_client, seal};
use crate::registry::servers::ServerRegistry;
use crate::types::SessionAudience;

const CONNECT_FLOW_SEAL_DESCRIPTION: &str = "the gateway could not seal the connect flow";
const PKCE_DESCRIPTION: &str =
    "PKCE is required: send code_challenge with code_challenge_method=S256";
const REDIRECT_URI_DESCRIPTION: &str = "redirect_uri is not registered for this client";

#[derive(Clone, Copy, Debug)]
pub struct AuthorizeRequest<'request> {
    pub client_id: &'request str,
    pub redirect_uri: &'request str,
    pub state: &'request str,
    pub code_challenge: Option<&'request str>,
    pub code_challenge_method: Option<&'request str>,
    pub response_type: Option<&'request str>,
}

pub struct NewConnectFlow<'request> {
    pub user_id: &'request str,
    pub client_id: &'request str,
    pub redirect_uri: &'request str,
    pub state: &'request str,
    pub code_challenge: String,
    pub resource_server_id: Option<String>,
    pub audience: Option<SessionAudience>,
}

pub struct AggregateAuthorize<'request> {
    pub view: &'request RequestView,
    pub request: &'request AuthorizeRequest<'request>,
    pub session_user_id: Option<&'request str>,
    pub resource: Option<&'request str>,
    pub now: i64,
}

#[must_use]
pub fn rejected_authorize_request(
    request: &AuthorizeRequest<'_>,
    cipher: &DbCipher,
) -> Option<OauthError> {
    let Some(client) = open_gateway_dcr_client(request.client_id, cipher) else {
        return Some(oauth_error(
            400,
            InvalidClient,
            "unknown or malformed client_id",
        ));
    };
    if !client
        .redirect_uris
        .iter()
        .any(|uri| uri == request.redirect_uri)
    {
        return Some(oauth_error(400, InvalidRequest, REDIRECT_URI_DESCRIPTION));
    }
    if request.response_type != Some("code") {
        return Some(oauth_error(
            400,
            UnsupportedResponseType,
            "response_type must be 'code'",
        ));
    }
    if request.code_challenge.is_none_or(str::is_empty)
        || request.code_challenge_method != Some("S256")
    {
        return Some(oauth_error(400, InvalidRequest, PKCE_DESCRIPTION));
    }
    if request.state.chars().count() > MAX_STATE_LENGTH {
        let description = format!("state must be at most {MAX_STATE_LENGTH} characters");
        return Some(oauth_error(400, InvalidRequest, &description));
    }
    None
}

#[must_use]
pub fn login_redirect(base_url: &str, path: &str, query: Option<&str>) -> GatewayReply {
    let return_to = relative_request_url(path, query);
    let location = format!(
        "{base_url}/sso/key/generate?{}",
        urlencode(&[("return_to".to_owned(), return_to)])
    );
    GatewayReply::redirect(location)
}

#[must_use]
pub fn new_connect_flow(
    fields: NewConnectFlow<'_>,
    tokens: &dyn FlowTokenSource,
    now: i64,
) -> ConnectFlow {
    ConnectFlow {
        user_id: fields.user_id.to_owned(),
        client_id: fields.client_id.to_owned(),
        redirect_uri: fields.redirect_uri.to_owned(),
        state: fields.state.to_owned(),
        code_challenge: fields.code_challenge,
        jti: tokens.fresh_token(),
        exp: now.saturating_add(CONNECT_FLOW_TTL_SECONDS),
        resource_server_id: fields.resource_server_id,
        audience: fields.audience,
    }
}

pub fn set_flow_cookie(
    handle: &str,
    flow: &ConnectFlow,
    base_url: &str,
    cipher: &DbCipher,
) -> Result<FlowCookie, OauthError> {
    let Ok(value) = seal("", flow, cipher) else {
        return Err(oauth_error(500, ServerError, CONNECT_FLOW_SEAL_DESCRIPTION));
    };
    let (path, secure) = cookie_path_and_secure(base_url)?;
    Ok(FlowCookie {
        name: flow_cookie_name(handle),
        value,
        max_age_seconds: CONNECT_FLOW_TTL_SECONDS,
        path,
        secure,
    })
}

#[must_use]
pub fn aggregate_authorize(
    call: &AggregateAuthorize<'_>,
    registry: &ServerRegistry,
    cipher: &DbCipher,
    tokens: &dyn FlowTokenSource,
) -> GatewayReply {
    GatewayReply::from(aggregate_reply(call, registry, cipher, tokens))
}

fn aggregate_reply(
    call: &AggregateAuthorize<'_>,
    registry: &ServerRegistry,
    cipher: &DbCipher,
    tokens: &dyn FlowTokenSource,
) -> Result<GatewayReply, OauthError> {
    if let Some(error) = rejected_authorize_request(call.request, cipher) {
        return Err(error);
    }
    let Some(user_id) = call.session_user_id else {
        return Ok(login_redirect(
            &call.view.base_url,
            &call.view.path,
            call.view.query.as_deref(),
        ));
    };
    let scoped = resolve_scoped_resource_server(registry, &call.view.base_url, call.resource);
    let handle = tokens.fresh_token();
    let flow = new_connect_flow(
        NewConnectFlow {
            user_id,
            client_id: call.request.client_id,
            redirect_uri: call.request.redirect_uri,
            state: call.request.state,
            code_challenge: call.request.code_challenge.unwrap_or_default().to_owned(),
            resource_server_id: scoped.map(|server| server.fields.server_id.clone()),
            audience: None,
        },
        tokens,
        call.now,
    );
    let cookie = set_flow_cookie(&handle, &flow, &call.view.base_url, cipher)?;
    let connect_url = append_query_params(
        &format!("{}/ui/connect", call.view.base_url),
        &[("connect_flow".to_owned(), handle)],
    )?;
    Ok(GatewayReply::Redirect {
        location: connect_url,
        set_cookie: Some(Box::new(cookie)),
        delete_cookie: None,
    })
}
