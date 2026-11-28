mod complete;
mod describe;
mod tests;

use betterlite_db::stored_cipher::DbCipher;

use self::complete::completed;
use self::describe::describe_opened_flow;
use crate::gateway_authorize::no_cache_headers;
use crate::gateway_authorize::oauth_error;
use crate::gateway_authorize::reply::{FlowTokenSource, GatewayReply, RequestView};
use crate::gateway_authorize::url_parts::flow_cookie_name;
use crate::gateway_dcr::contracts::{
    CONNECT_FLOW_DEBUG_KEY,
    // ast-grep-ignore: no-long-names
    MANUAL_DELIVERY_AUTH_CODE_TTL_SECONDS,
};
use crate::gateway_dcr::models::ConnectFlow;
use crate::gateway_dcr::oauth_error::OauthError;
use crate::gateway_dcr::oauth_error::OauthErrorCode::{
    AccessDenied, InvalidRequest, LoginRequired,
};
use crate::gateway_dcr::ports::{LookupServerReachability, LookupVendorCredential};
use crate::gateway_dcr::sealed::open_sealed;
use crate::gateway_dcr::single_use::{ClaimBackend, SingleUseGuard};
use crate::registry::servers::ServerRegistry;

const UNKNOWN_FLOW_DESCRIPTION: &str = "unknown or expired connect flow";

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

pub struct ConnectPorts<'ports, V: ?Sized, R: ?Sized> {
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
        return Err(oauth_error(400, InvalidRequest, UNKNOWN_FLOW_DESCRIPTION));
    };
    let Some(flow) = open_sealed::<ConnectFlow>(sealed, "", cipher, CONNECT_FLOW_DEBUG_KEY) else {
        return Err(oauth_error(400, InvalidRequest, UNKNOWN_FLOW_DESCRIPTION));
    };
    if now >= flow.exp {
        return Err(oauth_error(400, InvalidRequest, UNKNOWN_FLOW_DESCRIPTION));
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

pub async fn describe_connect_flow<V, R>(
    call: &ConnectFlowCall<'_>,
    ports: &ConnectPorts<'_, V, R>,
    cipher: &DbCipher,
) -> GatewayReply
where
    V: LookupVendorCredential + ?Sized,
    R: LookupServerReachability + ?Sized,
{
    let flow = match open_flow_for(
        call.view,
        call.handle,
        call.session_user_id,
        call.now,
        cipher,
    ) {
        Ok(flow) => flow,
        Err(error) => return GatewayReply::Error(error),
    };
    match describe_opened_flow(&flow, ports).await {
        Ok(description) => GatewayReply::Json {
            body: Box::new(description),
            headers: no_cache_headers(),
        },
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
        Err(error) => GatewayReply::Error(error),
        Ok(reply) => reply,
    }
}

#[must_use]
pub fn manual_delivery_page(callback_url: &str) -> String {
    let safe_url = escape_html(callback_url);
    // ast-grep-ignore: no-long-names
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

// byok_authorization_html's copy is private and outside this task's scope; html.escape(quote=True) needs these five.
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}
