use betterlite_db::stored_cipher::DbCipher;

use super::{
    CompleteFlowRequest, ConnectFlowCall, ConnectPorts, FlowForm, manual_delivery_page,
    open_flow_for,
};
use crate::gateway_authorize::is_loopback_uri;
use crate::gateway_authorize::no_cache_headers;
use crate::gateway_authorize::oauth_error;
use crate::gateway_authorize::reply::{
    ConnectFlowState, FlowCookieDeletion, FlowTokenSource, GatewayReply,
};
use crate::gateway_authorize::url_parts::{
    append_query_params, cookie_path_and_secure, flow_cookie_name,
};
use crate::gateway_dcr::contracts::{
    CLAIM_TTL_BUFFER_SECONDS,
    CONNECT_FLOW_TTL_SECONDS,
    FLOW_CLAIM_KEY_PREFIX,
    GATEWAY_AUTH_CODE_PREFIX,
    GATEWAY_AUTH_CODE_TTL_SECONDS,
    // ast-grep-ignore: no-long-names
    MANUAL_DELIVERY_AUTH_CODE_TTL_SECONDS,
    PROXY_API_AUDIENCE,
};
use crate::gateway_dcr::models::{ConnectFlow, GatewayAuthCode};
use crate::gateway_dcr::oauth_error::OauthError;
use crate::gateway_dcr::oauth_error::OauthErrorCode::{InvalidRequest, ServerError};
use crate::gateway_dcr::ports::{LookupServerReachability, LookupVendorCredential};
use crate::gateway_dcr::sealed::seal;
use crate::gateway_dcr::single_use::{ClaimBackend, SingleUseGuard, claim_refusal};

const CODE_SEAL_DESCRIPTION: &str = "the gateway could not seal the authorization code";

pub(super) async fn completed<G, V, R>(
    call: &CompleteFlowRequest<'_>,
    ports: &ConnectPorts<'_, V, R>,
    cipher: &DbCipher,
    guard: &SingleUseGuard<G>,
    tokens: &dyn FlowTokenSource,
) -> Result<GatewayReply, OauthError>
where
    G: ClaimBackend,
    V: LookupVendorCredential + ?Sized,
    R: LookupServerReachability + ?Sized,
{
    validate_form(&call.form)?;
    let flow = open_flow_for(
        call.call.view,
        call.call.handle,
        call.call.session_user_id,
        call.call.now,
        cipher,
    )?;
    if call.form.decision != Some("deny") {
        scoped_gates(&flow, ports).await?;
    }
    claim_once(guard, &flow).await?;
    finish(call, &flow, cipher, tokens)
}

async fn claim_once<G>(guard: &SingleUseGuard<G>, flow: &ConnectFlow) -> Result<(), OauthError>
where
    G: ClaimBackend,
{
    let key = format!("{FLOW_CLAIM_KEY_PREFIX}{}", flow.jti);
    let ttl = CONNECT_FLOW_TTL_SECONDS.saturating_add(CLAIM_TTL_BUFFER_SECONDS);
    let refusal = claim_refusal(
        guard.claim(&key, ttl).await,
        oauth_error(
            400,
            InvalidRequest,
            "this connect flow was already completed; restart the connection",
        ),
    );
    refusal.map_or(Ok(()), Err)
}

fn validate_form(form: &FlowForm<'_>) -> Result<(), OauthError> {
    if !matches!(form.delivery, None | Some("redirect" | "manual")) {
        return Err(oauth_error(
            400,
            InvalidRequest,
            "delivery must be 'redirect' or 'manual'",
        ));
    }
    if !matches!(form.decision, None | Some("approve" | "deny")) {
        return Err(oauth_error(
            400,
            InvalidRequest,
            "decision must be 'approve' or 'deny'",
        ));
    }
    Ok(())
}

async fn scoped_gates<V, R>(
    flow: &ConnectFlow,
    ports: &ConnectPorts<'_, V, R>,
) -> Result<(), OauthError>
where
    V: LookupVendorCredential + ?Sized,
    R: LookupServerReachability + ?Sized,
{
    let opened = super::describe::describe_opened_flow(flow, ports).await?;
    if opened.state == ConnectFlowState::Stale {
        return Err(oauth_error(
            400,
            InvalidRequest,
            "the requested MCP server is no longer available",
        ));
    }
    if opened.connected == Some(false) {
        return Err(oauth_error(
            400,
            InvalidRequest,
            "authorize the requested MCP server before finishing",
        ));
    }
    Ok(())
}

fn finish(
    call: &CompleteFlowRequest<'_>,
    flow: &ConnectFlow,
    cipher: &DbCipher,
    tokens: &dyn FlowTokenSource,
) -> Result<GatewayReply, OauthError> {
    let reply = if call.form.decision == Some("deny") {
        denied_reply(flow)?
    } else {
        approved_reply(&call.call, &call.form, flow, cipher, tokens)?
    };
    let (path, secure) = cookie_path_and_secure(&call.call.view.base_url)?;
    let deletion = FlowCookieDeletion {
        name: flow_cookie_name(call.call.handle),
        path,
        secure,
    };
    Ok(with_deletion(reply, deletion))
}

fn with_deletion(reply: GatewayReply, deletion: FlowCookieDeletion) -> GatewayReply {
    let delete_cookie = Some(Box::new(deletion));
    match reply {
        GatewayReply::Redirect {
            location,
            set_cookie,
            ..
        } => GatewayReply::Redirect {
            location,
            set_cookie,
            delete_cookie,
        },
        GatewayReply::Html {
            body,
            headers,
            set_cookie,
            ..
        } => GatewayReply::Html {
            body,
            headers,
            set_cookie,
            delete_cookie,
        },
        other @ (GatewayReply::Json { .. } | GatewayReply::Error(_)) => other,
    }
}

fn denied_reply(flow: &ConnectFlow) -> Result<GatewayReply, OauthError> {
    let mut params = vec![("error".to_owned(), "access_denied".to_owned())];
    params.extend(state_param(flow));
    let location = append_query_params(&flow.redirect_uri, &params)?;
    Ok(GatewayReply::redirect(location))
}

fn approved_reply(
    call: &ConnectFlowCall<'_>,
    form: &FlowForm<'_>,
    flow: &ConnectFlow,
    cipher: &DbCipher,
    tokens: &dyn FlowTokenSource,
) -> Result<GatewayReply, OauthError> {
    let manual = form.delivery == Some("manual") && is_loopback_uri(&flow.redirect_uri);
    let code = auth_code(call, form, flow, tokens, code_ttl(manual));
    let Ok(sealed) = seal(GATEWAY_AUTH_CODE_PREFIX, &code, cipher) else {
        return Err(oauth_error(500, ServerError, CODE_SEAL_DESCRIPTION));
    };
    let callback_url = callback_url(flow, &sealed)?;
    if manual {
        return Ok(GatewayReply::Html {
            body: manual_delivery_page(&callback_url),
            headers: no_cache_headers(),
            set_cookie: None,
            delete_cookie: None,
        });
    }
    Ok(GatewayReply::redirect(callback_url))
}

const fn code_ttl(manual: bool) -> i64 {
    if manual {
        // ast-grep-ignore: no-long-names
        MANUAL_DELIVERY_AUTH_CODE_TTL_SECONDS
    } else {
        GATEWAY_AUTH_CODE_TTL_SECONDS
    }
}

fn auth_code(
    call: &ConnectFlowCall<'_>,
    form: &FlowForm<'_>,
    flow: &ConnectFlow,
    tokens: &dyn FlowTokenSource,
    ttl: i64,
) -> GatewayAuthCode {
    GatewayAuthCode {
        user_id: flow.user_id.clone(),
        client_id: flow.client_id.clone(),
        redirect_uri: flow.redirect_uri.clone(),
        code_challenge: flow.code_challenge.clone(),
        jti: tokens.fresh_token(),
        iat: call.now,
        exp: call.now.saturating_add(ttl),
        resource_server_id: flow.resource_server_id.clone(),
        audience: flow.audience,
        team_id: code_team_id(flow, form.team_id),
    }
}

fn callback_url(flow: &ConnectFlow, code: &str) -> Result<String, OauthError> {
    let mut params = vec![("code".to_owned(), code.to_owned())];
    params.extend(state_param(flow));
    Ok(append_query_params(&flow.redirect_uri, &params)?)
}

fn code_team_id(flow: &ConnectFlow, team_id: Option<&str>) -> Option<String> {
    if flow.audience != Some(PROXY_API_AUDIENCE) {
        return None;
    }
    team_id.filter(|value| !value.is_empty()).map(str::to_owned)
}

fn state_param(flow: &ConnectFlow) -> Vec<(String, String)> {
    if flow.state.is_empty() {
        Vec::new()
    } else {
        vec![("state".to_owned(), flow.state.clone())]
    }
}
