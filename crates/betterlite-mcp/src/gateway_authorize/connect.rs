mod complete;
mod describe;
mod tests;

use betterlite_db::stored_cipher::DbCipher;

use self::complete::completed;
use self::describe::described_reply;
use crate::gateway_authorize::oauth_error;
use crate::gateway_authorize::reply::{
    ConnectFlowDescription, ConnectFlowState, FlowTokenSource, GatewayReply, RequestView,
};
use crate::gateway_authorize::scope::{FlowTarget, flow_target};
use crate::gateway_authorize::url_parts::{flow_cookie_name, origin_only};
use crate::gateway_dcr::contracts::{
    CONNECT_FLOW_DEBUG_KEY, DB_UNAVAILABLE_DESCRIPTION, MANUAL_DELIVERY_AUTH_CODE_TTL_SECONDS,
};
use crate::gateway_dcr::models::ConnectFlow;
use crate::gateway_dcr::oauth_error::OauthError;
use crate::gateway_dcr::oauth_error::OauthErrorCode::{
    AccessDenied, InvalidRequest, LoginRequired, TemporarilyUnavailable,
};
use crate::gateway_dcr::ports::{
    LookupServerReachability, LookupVendorCredential, VendorCredentialState,
};
use crate::gateway_dcr::sealed::open_sealed;
use crate::gateway_dcr::single_use::{ClaimBackend, SingleUseGuard};
use crate::registry::servers::ServerRegistry;
use crate::server_record::MCPServer;

pub struct ConnectFlowCall<'call> {
    pub view: &'call RequestView,
    pub handle: &'call str,
    pub session_user_id: Option<&'call str>,
    pub now: i64,
}

pub struct FlowForm<'form> {
    pub delivery: Option<&'form str>,
    pub team_id: Option<&'form str>,
    pub decision: Option<&'form str>,
}

pub struct CompleteFlowRequest<'call> {
    pub call: ConnectFlowCall<'call>,
    pub form: FlowForm<'call>,
}

pub struct ConnectPorts<'ports, V, R> {
    pub registry: &'ports ServerRegistry,
    pub vendor: &'ports V,
    pub reachability: &'ports R,
}

pub fn open_flow_for(
    view: &RequestView,
    handle: &str,
    session_user_id: Option<&str>,
    now: i64,
    cipher: &DbCipher,
) -> Result<ConnectFlow, OauthError> {
    let Some(sealed) = view.cookie(&flow_cookie_name(handle)) else {
        return Err(unknown_connect_flow());
    };
    let Some(flow) = open_sealed::<ConnectFlow>(sealed, "", cipher, CONNECT_FLOW_DEBUG_KEY) else {
        return Err(unknown_connect_flow());
    };
    if now >= flow.exp {
        return Err(unknown_connect_flow());
    }
    let Some(user_id) = session_user_id else {
        return Err(oauth_error(
            401,
            LoginRequired,
            "sign in to LiteLLM to finish connecting",
        ));
    };
    if user_id != flow.user_id {
        return Err(oauth_error(
            403,
            AccessDenied,
            "the signed-in user does not match this connect flow",
        ));
    }
    Ok(flow)
}

pub async fn describe_opened_flow<V, R>(
    flow: &ConnectFlow,
    ports: &ConnectPorts<'_, V, R>,
) -> Result<ConnectFlowDescription, OauthError>
where
    V: LookupVendorCredential + ?Sized,
    R: LookupServerReachability + ?Sized,
{
    let target = flow_target(ports.registry, flow, ports.reachability).await;
    let origin = origin_only(&flow.redirect_uri)?;
    match target {
        FlowTarget::Interactive(server) => {
            let connected = match ports
                .vendor
                .lookup(&flow.user_id, &server.fields.server_id)
                .await
            {
                VendorCredentialState::Unavailable => {
                    return Err(oauth_error(
                        503,
                        TemporarilyUnavailable,
                        DB_UNAVAILABLE_DESCRIPTION,
                    ));
                }
                VendorCredentialState::Present => true,
                VendorCredentialState::Absent => false,
            };
            Ok(flow_description(
                ConnectFlowState::Interactive,
                origin,
                Some(server),
                Some(connected),
            ))
        }
        FlowTarget::M2m(server) => Ok(flow_description(
            ConnectFlowState::M2m,
            origin,
            Some(server),
            Some(true),
        )),
        FlowTarget::Unscoped => Ok(flow_description(
            ConnectFlowState::Unscoped,
            origin,
            None,
            None,
        )),
        FlowTarget::Stale => Ok(flow_description(
            ConnectFlowState::Stale,
            origin,
            None,
            None,
        )),
    }
}

pub async fn describe_connect_flow<V, R>(
    call: &ConnectFlowCall<'_>,
    ports: &ConnectPorts<'_, V, R>,
    cipher: &DbCipher,
) -> GatewayReply
where
    V: LookupVendorCredential + ?Sized,
    R: LookupServerReachability + ?Sized,
{
    match described_reply(call, ports, cipher).await {
        Ok(reply) => reply,
        Err(error) => GatewayReply::Error(error),
    }
}

pub async fn complete_connect_flow<G, V, R>(
    call: &CompleteFlowRequest<'_>,
    ports: &ConnectPorts<'_, V, R>,
    cipher: &DbCipher,
    guard: &SingleUseGuard<G>,
    tokens: &dyn FlowTokenSource,
) -> GatewayReply
where
    G: ClaimBackend,
    V: LookupVendorCredential + ?Sized,
    R: LookupServerReachability + ?Sized,
{
    match completed(call, ports, cipher, guard, tokens).await {
        Ok(reply) => reply,
        Err(error) => GatewayReply::Error(error),
    }
}

fn flow_description(
    state: ConnectFlowState,
    client_origin: String,
    server: Option<&MCPServer>,
    connected: Option<bool>,
) -> ConnectFlowDescription {
    ConnectFlowDescription {
        state,
        client_origin,
        server_id: server.map(|server| server.fields.server_id.clone()),
        server_name: server.and_then(server_display_name),
        connected,
    }
}

fn server_display_name(server: &MCPServer) -> Option<String> {
    let fields = &server.fields;
    [
        fields.server_name.as_deref(),
        fields.alias.as_deref(),
        Some(fields.name.as_str()),
    ]
    .into_iter()
    .flatten()
    .find(|value| !value.is_empty())
    .map(str::to_owned)
}

#[must_use]
pub fn manual_delivery_page(callback_url: &str) -> String {
    let safe_url = escape_html(callback_url);
    let minutes = MANUAL_DELIVERY_AUTH_CODE_TTL_SECONDS.div_euclid(60);
    format!(
        "<html><head><title>Finish connecting</title></head><body><h2>Almost done</h2>\
         <p>Your MCP client runs on a different machine, so this browser cannot deliver the \
         authorization code to it. On the machine where the client runs, paste this URL into \
         the client's prompt (Claude Code accepts the pasted callback URL), or pass it as the \
         quoted argument of a curl command from that machine's terminal:</p>\
         <p><input type=\"text\" value=\"{safe_url}\" readonly size=\"100\" onclick=\"this.select()\"></p>\
         <p>The code is single-use and expires in {minutes} minutes. You can close this window \
         once the client confirms it is connected.</p></body></html>"
    )
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

fn unknown_connect_flow() -> OauthError {
    oauth_error(400, InvalidRequest, "unknown or expired connect flow")
}
