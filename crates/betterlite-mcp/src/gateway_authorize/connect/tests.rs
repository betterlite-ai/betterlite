#![cfg(test)]

use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use percent_encoding::percent_decode_str;

use super::*;
use crate::gateway_authorize::no_cache_headers;
use crate::gateway_authorize::reply::{ConnectFlowDescription, ConnectFlowState};
use crate::gateway_authorize::tests::support::{
    CALLBACK, M2M_SERVER, NoBackend, OAUTH_SERVER, Reach, SALT, ScriptedTokens, Vendor, cipher,
    flow, guard, registry, sealed_view, view,
};
use crate::gateway_authorize::url_parts::UNREADABLE_URL_DESCRIPTION;
use crate::gateway_dcr::contracts::{
    AUTH_CODE_DEBUG_KEY, CLAIM_TTL_BUFFER_SECONDS, CONNECT_FLOW_TTL_SECONDS,
    DB_UNAVAILABLE_DESCRIPTION, FLOW_CLAIM_KEY_PREFIX, GATEWAY_AUTH_CODE_PREFIX,
    PROXY_API_AUDIENCE,
};
use crate::gateway_dcr::models::{ConnectFlow, GatewayAuthCode};
use crate::gateway_dcr::oauth_error::OauthErrorCode;
use crate::gateway_dcr::ports::VendorCredentialState;
use crate::gateway_dcr::sealed::open_sealed;
use crate::gateway_dcr::single_use::{
    CLAIM_UNAVAILABLE_DESCRIPTION, ClaimAuthority, ClaimBackendError,
};

const PRETTY_SERVER: &str = r#"{"server_id":"srv-n","name":"delta","server_name":"Pretty","transport":"sse","auth_type":"oauth2","oauth2_flow":"authorization_code"}"#;
const ALIASED_SERVER: &str = r#"{"server_id":"srv-m","name":"epsilon","server_name":"","alias":"Alias","transport":"sse","auth_type":"oauth2","oauth2_flow":"authorization_code"}"#;
const HOSTED_CALLBACK: &str = "https://claude.ai/cb";
const UNKNOWN_FLOW: &str = "unknown or expired connect flow";

struct Ports {
    servers: ServerRegistry,
    vendor: Vendor,
    reach: Reach,
}

impl Ports {
    fn new(servers: &[&str], vendor: VendorCredentialState, reachable: bool) -> Self {
        Self {
            servers: registry(servers),
            vendor: Vendor(vendor),
            reach: Reach(reachable),
        }
    }

    const fn ports(&self) -> ConnectPorts<'_, Vendor, Reach> {
        ConnectPorts {
            registry: &self.servers,
            vendor: &self.vendor,
            reachability: &self.reach,
        }
    }
}

fn minted() -> ConnectFlow {
    ConnectFlow {
        exp: 1_000 + CONNECT_FLOW_TTL_SECONDS,
        ..flow()
    }
}

fn minted_for(server_id: &str) -> ConnectFlow {
    ConnectFlow {
        resource_server_id: Some(server_id.to_owned()),
        ..minted()
    }
}

const fn approve_form() -> FlowForm<'static> {
    FlowForm {
        delivery: None,
        team_id: None,
        decision: Some("approve"),
    }
}

fn check_error(error: &OauthError, status: u16, code: OauthErrorCode, description: &str) {
    assert_eq!(error.status, status);
    assert_eq!(error.error, code);
    assert_eq!(error.description, description);
}

fn error_of(reply: GatewayReply) -> OauthError {
    let GatewayReply::Error(error) = reply else {
        panic!("expected an error");
    };
    error
}

fn description_of(reply: GatewayReply) -> ConnectFlowDescription {
    let GatewayReply::Json { body, headers } = reply else {
        panic!("expected a description");
    };
    assert_eq!(headers, no_cache_headers());
    *body
}

fn callback_code(location: &str) -> String {
    let (_head, rest) = location.split_once("code=").expect("code parameter");
    rest.split_once('&')
        .map_or(rest, |(code, _tail)| code)
        .to_owned()
}

fn page_url(body: &str) -> String {
    let (_head, rest) = body.split_once("value=\"").expect("url input");
    rest.split_once('"').expect("closing quote").0.to_owned()
}

fn opened_code(code: &str, db_cipher: &DbCipher) -> GatewayAuthCode {
    let decoded = percent_decode_str(code)
        .decode_utf8()
        .expect("decoded code");
    open_sealed::<GatewayAuthCode>(
        &decoded,
        GATEWAY_AUTH_CODE_PREFIX,
        db_cipher,
        AUTH_CODE_DEBUG_KEY,
    )
    .expect("sealed code")
}

const fn complete_request<'call>(
    request_view: &'call RequestView,
    decision: Option<&'call str>,
    delivery: Option<&'call str>,
    team_id: Option<&'call str>,
) -> CompleteFlowRequest<'call> {
    CompleteFlowRequest {
        call: ConnectFlowCall {
            view: request_view,
            handle: "handle-1",
            session_user_id: Some("alice"),
            now: 1_000,
        },
        form: FlowForm {
            delivery,
            team_id,
            decision,
        },
    }
}

async fn describe_reply(
    connect_flow: &ConnectFlow,
    ports: &ConnectPorts<'_, Vendor, Reach>,
) -> GatewayReply {
    let request_view = sealed_view(connect_flow, "handle-1");
    let call = ConnectFlowCall {
        view: &request_view,
        handle: "handle-1",
        session_user_id: Some("alice"),
        now: 1_000,
    };
    describe_connect_flow(&call, ports, &cipher(SALT)).await
}

async fn complete_with(
    connect_flow: &ConnectFlow,
    ports: &Ports,
    form: FlowForm<'_>,
) -> GatewayReply {
    let request_view = sealed_view(connect_flow, "handle-1");
    let request = CompleteFlowRequest {
        call: ConnectFlowCall {
            view: &request_view,
            handle: "handle-1",
            session_user_id: Some("alice"),
            now: 1_000,
        },
        form,
    };
    let tokens = ScriptedTokens::new(&["code-jti-1"]);
    complete_connect_flow(&request, &ports.ports(), &cipher(SALT), &guard(), &tokens).await
}

#[test]
fn an_open_flow_needs_its_cookie() {
    let db_cipher = cipher(SALT);
    let minted_flow = minted();
    let sealed = sealed_view(&minted_flow, "handle-1");
    check_error(
        &open_flow_for(&sealed, "handle-9", Some("alice"), 1_000, &db_cipher).unwrap_err(),
        400,
        InvalidRequest,
        UNKNOWN_FLOW,
    );
    let no_cookie = view("/authorize/complete", None, &[]);
    check_error(
        &open_flow_for(&no_cookie, "handle-1", Some("alice"), 1_000, &db_cipher).unwrap_err(),
        400,
        InvalidRequest,
        UNKNOWN_FLOW,
    );
    let junk = view(
        "/authorize/complete",
        None,
        &[("mcp_connect_flow_handle-1", "junk")],
    );
    check_error(
        &open_flow_for(&junk, "handle-1", Some("alice"), 1_000, &db_cipher).unwrap_err(),
        400,
        InvalidRequest,
        UNKNOWN_FLOW,
    );
}

#[test]
fn an_open_flow_expires_at_its_exact_second() {
    let db_cipher = cipher(SALT);
    let minted_flow = minted();
    let sealed = sealed_view(&minted_flow, "handle-1");
    assert_eq!(
        open_flow_for(&sealed, "handle-1", Some("alice"), 999, &db_cipher).unwrap(),
        minted_flow
    );
    assert_eq!(
        open_flow_for(&sealed, "handle-1", Some("alice"), 1_599, &db_cipher).unwrap(),
        minted_flow
    );
    check_error(
        &open_flow_for(&sealed, "handle-1", Some("alice"), 1_600, &db_cipher).unwrap_err(),
        400,
        InvalidRequest,
        UNKNOWN_FLOW,
    );
}

#[test]
fn an_open_flow_needs_the_same_signed_in_user() {
    let db_cipher = cipher(SALT);
    let sealed = sealed_view(&minted(), "handle-1");
    check_error(
        &open_flow_for(&sealed, "handle-1", None, 1_000, &db_cipher).unwrap_err(),
        401,
        LoginRequired,
        "sign in to LiteLLM to finish connecting",
    );
    check_error(
        &open_flow_for(&sealed, "handle-1", Some("bob"), 1_000, &db_cipher).unwrap_err(),
        403,
        AccessDenied,
        "the signed-in user does not match this connect flow",
    );
}

#[tokio::test]
async fn describe_reports_an_unscoped_flow() {
    let ports = Ports::new(&[], VendorCredentialState::Absent, true);
    let description = description_of(describe_reply(&minted(), &ports.ports()).await);
    assert_eq!(description.state, ConnectFlowState::Unscoped);
    assert_eq!(description.client_origin, "http://127.0.0.1:8080");
    assert_eq!(description.server_id, None);
    assert_eq!(description.server_name, None);
    assert_eq!(description.connected, None);
}

#[tokio::test]
async fn describe_reports_an_interactive_flow() {
    let absent = Ports::new(&[OAUTH_SERVER], VendorCredentialState::Absent, true);
    let offline = description_of(describe_reply(&minted_for("srv-a"), &absent.ports()).await);
    assert_eq!(offline.state, ConnectFlowState::Interactive);
    assert_eq!(offline.server_id.as_deref(), Some("srv-a"));
    assert_eq!(offline.server_name.as_deref(), Some("alpha"));
    assert_eq!(offline.connected, Some(false));
    let present = Ports::new(&[OAUTH_SERVER], VendorCredentialState::Present, true);
    let online = description_of(describe_reply(&minted_for("srv-a"), &present.ports()).await);
    assert_eq!(online.connected, Some(true));
}

#[tokio::test]
async fn describe_reports_an_m2m_flow() {
    let ports = Ports::new(&[M2M_SERVER], VendorCredentialState::Unavailable, true);
    let description = description_of(describe_reply(&minted_for("srv-b"), &ports.ports()).await);
    assert_eq!(description.state, ConnectFlowState::M2m);
    assert_eq!(description.server_id.as_deref(), Some("srv-b"));
    assert_eq!(description.server_name.as_deref(), Some("beta"));
    assert_eq!(description.connected, Some(true));
}

#[tokio::test]
async fn describe_reports_a_stale_flow() {
    let oauth = Ports::new(&[OAUTH_SERVER], VendorCredentialState::Absent, true);
    let missing = description_of(describe_reply(&minted_for("srv-z"), &oauth.ports()).await);
    assert_eq!(missing.state, ConnectFlowState::Stale);
    assert_eq!(missing.server_id, None);
    assert_eq!(missing.connected, None);
    let down = Ports::new(&[OAUTH_SERVER], VendorCredentialState::Absent, false);
    let stale = description_of(describe_reply(&minted_for("srv-a"), &down.ports()).await);
    assert_eq!(stale.state, ConnectFlowState::Stale);
}

#[tokio::test]
async fn describe_names_the_server_by_its_first_label() {
    let pretty = Ports::new(&[PRETTY_SERVER], VendorCredentialState::Absent, true);
    let named = description_of(describe_reply(&minted_for("srv-n"), &pretty.ports()).await);
    assert_eq!(named.server_name.as_deref(), Some("Pretty"));
    let aliased = Ports::new(&[ALIASED_SERVER], VendorCredentialState::Absent, true);
    let alias = description_of(describe_reply(&minted_for("srv-m"), &aliased.ports()).await);
    assert_eq!(alias.server_name.as_deref(), Some("Alias"));
}

#[tokio::test]
async fn describe_fails_when_the_vendor_lookup_is_unavailable() {
    let ports = Ports::new(&[OAUTH_SERVER], VendorCredentialState::Unavailable, true);
    let reply = describe_reply(&minted_for("srv-a"), &ports.ports()).await;
    check_error(
        &error_of(reply),
        503,
        OauthErrorCode::TemporarilyUnavailable,
        DB_UNAVAILABLE_DESCRIPTION,
    );
}

#[tokio::test]
async fn an_unreadable_redirect_uri_is_a_server_error() {
    let broken = ConnectFlow {
        redirect_uri: "http://[::1".to_owned(),
        ..minted()
    };
    let ports = Ports::new(&[], VendorCredentialState::Absent, true);
    let reply = describe_reply(&broken, &ports.ports()).await;
    check_error(
        &error_of(reply),
        500,
        OauthErrorCode::ServerError,
        UNREADABLE_URL_DESCRIPTION,
    );
}

#[tokio::test]
async fn a_bad_decision_is_refused_before_the_flow_opens() {
    let ports = Ports::new(&[], VendorCredentialState::Absent, true);
    let form = FlowForm {
        delivery: Some("redirect"),
        team_id: None,
        decision: Some("maybe"),
    };
    let reply = complete_with(&minted(), &ports, form).await;
    check_error(
        &error_of(reply),
        400,
        InvalidRequest,
        "decision must be 'approve' or 'deny'",
    );
}

#[tokio::test]
async fn a_bad_delivery_is_refused_before_the_flow_opens() {
    let ports = Ports::new(&[], VendorCredentialState::Absent, true);
    let form = FlowForm {
        delivery: Some("carrier-pigeon"),
        team_id: None,
        decision: Some("approve"),
    };
    let reply = complete_with(&minted(), &ports, form).await;
    check_error(
        &error_of(reply),
        400,
        InvalidRequest,
        "delivery must be 'redirect' or 'manual'",
    );
}

#[tokio::test]
async fn an_absent_form_reaches_the_flow_lookup() {
    let ports = Ports::new(&[], VendorCredentialState::Absent, true);
    let request_view = view("/authorize/complete", None, &[]);
    let flow_guard = guard();
    let tokens = ScriptedTokens::new(&[]);
    let request = complete_request(&request_view, None, None, None);
    let reply = complete_connect_flow(
        &request,
        &ports.ports(),
        &cipher(SALT),
        &flow_guard,
        &tokens,
    )
    .await;
    check_error(&error_of(reply), 400, InvalidRequest, UNKNOWN_FLOW);
}

#[tokio::test]
async fn a_deny_redirects_with_access_denied_and_clears_the_cookie() {
    let stale_flow = minted_for("srv-z");
    let request_view = sealed_view(&stale_flow, "handle-1");
    let ports = Ports::new(&[], VendorCredentialState::Absent, true);
    let flow_guard = guard();
    let tokens = ScriptedTokens::new(&[]);
    let request = complete_request(&request_view, Some("deny"), None, None);
    let reply = complete_connect_flow(
        &request,
        &ports.ports(),
        &cipher(SALT),
        &flow_guard,
        &tokens,
    )
    .await;
    let GatewayReply::Redirect {
        location,
        set_cookie,
        delete_cookie,
    } = reply
    else {
        panic!("expected a redirect");
    };
    assert_eq!(
        location,
        format!("{CALLBACK}?error=access_denied&state=st-1")
    );
    assert!(set_cookie.is_none());
    let deletion = delete_cookie.expect("cookie deletion");
    assert_eq!(deletion.name, "mcp_connect_flow_handle-1");
    assert_eq!(deletion.path, "/");
    assert!(deletion.secure);
}

#[tokio::test]
async fn an_approve_redirects_with_a_sealed_code() {
    let ports = Ports::new(&[], VendorCredentialState::Absent, true);
    let reply = complete_with(&minted(), &ports, approve_form()).await;
    let GatewayReply::Redirect {
        location,
        set_cookie,
        delete_cookie,
    } = reply
    else {
        panic!("expected a redirect");
    };
    assert!(set_cookie.is_none());
    assert!(delete_cookie.is_some());
    let code = callback_code(&location);
    assert_eq!(location, format!("{CALLBACK}?code={code}&state=st-1"));
    assert!(code.starts_with(GATEWAY_AUTH_CODE_PREFIX));
    let opened = opened_code(&code, &cipher(SALT));
    assert_eq!(opened.user_id, "alice");
    assert_eq!(opened.client_id, "llm_dcrc_client");
    assert_eq!(opened.redirect_uri, CALLBACK);
    assert_eq!(opened.code_challenge, "challenge-1");
    assert_eq!(opened.jti, "code-jti-1");
    assert_eq!(opened.iat, 1_000);
    assert_eq!(opened.exp, 1_120);
    assert_eq!(opened.resource_server_id, None);
    assert_eq!(opened.audience, None);
    assert_eq!(opened.team_id, None);
}

#[tokio::test]
async fn a_missing_decision_approves() {
    let ports = Ports::new(&[], VendorCredentialState::Absent, true);
    let form = FlowForm {
        delivery: None,
        team_id: None,
        decision: None,
    };
    let reply = complete_with(&minted(), &ports, form).await;
    let GatewayReply::Redirect { location, .. } = reply else {
        panic!("expected a redirect");
    };
    assert!(location.starts_with(&format!("{CALLBACK}?code=")));
}

#[tokio::test]
async fn an_empty_flow_state_is_omitted_from_the_callback() {
    let empty_state = ConnectFlow {
        state: String::new(),
        ..minted()
    };
    let ports = Ports::new(&[], VendorCredentialState::Absent, true);
    let reply = complete_with(&empty_state, &ports, approve_form()).await;
    let GatewayReply::Redirect { location, .. } = reply else {
        panic!("expected a redirect");
    };
    assert!(location.starts_with(&format!("{CALLBACK}?code=")));
    assert!(!location.contains("state="));
}

#[tokio::test]
async fn a_team_rides_on_a_proxy_api_code() {
    let proxy = ConnectFlow {
        audience: Some(PROXY_API_AUDIENCE),
        ..minted()
    };
    let ports = Ports::new(&[], VendorCredentialState::Absent, true);
    let form = FlowForm {
        delivery: None,
        team_id: Some("team-1"),
        decision: Some("approve"),
    };
    let reply = complete_with(&proxy, &ports, form).await;
    let GatewayReply::Redirect { location, .. } = reply else {
        panic!("expected a redirect");
    };
    let opened = opened_code(&callback_code(&location), &cipher(SALT));
    assert_eq!(opened.team_id.as_deref(), Some("team-1"));
}

#[tokio::test]
async fn a_blank_team_is_absent_from_the_code() {
    let proxy = ConnectFlow {
        audience: Some(PROXY_API_AUDIENCE),
        ..minted()
    };
    let ports = Ports::new(&[], VendorCredentialState::Absent, true);
    let form = FlowForm {
        delivery: None,
        team_id: Some(""),
        decision: Some("approve"),
    };
    let reply = complete_with(&proxy, &ports, form).await;
    let GatewayReply::Redirect { location, .. } = reply else {
        panic!("expected a redirect");
    };
    let opened = opened_code(&callback_code(&location), &cipher(SALT));
    assert_eq!(opened.team_id, None);
}

#[tokio::test]
async fn a_mcp_session_code_never_gets_a_team() {
    let ports = Ports::new(&[], VendorCredentialState::Absent, true);
    let form = FlowForm {
        delivery: None,
        team_id: Some("team-1"),
        decision: Some("approve"),
    };
    let reply = complete_with(&minted(), &ports, form).await;
    let GatewayReply::Redirect { location, .. } = reply else {
        panic!("expected a redirect");
    };
    let opened = opened_code(&callback_code(&location), &cipher(SALT));
    assert_eq!(opened.team_id, None);
}

#[tokio::test]
async fn a_manual_delivery_renders_the_copy_paste_page() {
    let ports = Ports::new(&[], VendorCredentialState::Absent, true);
    let form = FlowForm {
        delivery: Some("manual"),
        team_id: None,
        decision: Some("approve"),
    };
    let reply = complete_with(&minted(), &ports, form).await;
    let GatewayReply::Html {
        body,
        headers,
        set_cookie,
        delete_cookie,
    } = reply
    else {
        panic!("expected the manual page");
    };
    assert_eq!(headers, no_cache_headers());
    assert!(set_cookie.is_none());
    assert!(delete_cookie.is_some());
    assert!(body.contains("expires in 5 minutes"));
    let url = page_url(&body);
    assert!(url.starts_with(&format!("{CALLBACK}?code={GATEWAY_AUTH_CODE_PREFIX}")));
    assert!(url.ends_with("&amp;state=st-1"));
    let opened = opened_code(&callback_code(&url), &cipher(SALT));
    assert_eq!(opened.exp, 1_300);
}

#[tokio::test]
async fn a_manual_delivery_to_a_hosted_callback_is_a_redirect() {
    let hosted = ConnectFlow {
        redirect_uri: HOSTED_CALLBACK.to_owned(),
        ..minted()
    };
    let ports = Ports::new(&[], VendorCredentialState::Absent, true);
    let form = FlowForm {
        delivery: Some("manual"),
        team_id: None,
        decision: Some("approve"),
    };
    let reply = complete_with(&hosted, &ports, form).await;
    let GatewayReply::Redirect { location, .. } = reply else {
        panic!("expected a redirect");
    };
    assert!(location.starts_with(&format!("{HOSTED_CALLBACK}?code=")));
}

#[tokio::test]
async fn a_stale_scoped_flow_cannot_be_completed() {
    let ports = Ports::new(&[], VendorCredentialState::Absent, true);
    let reply = complete_with(&minted_for("srv-z"), &ports, approve_form()).await;
    check_error(
        &error_of(reply),
        400,
        InvalidRequest,
        "the requested MCP server is no longer available",
    );
}

#[tokio::test]
async fn an_unconnected_scoped_flow_cannot_be_completed() {
    let ports = Ports::new(&[OAUTH_SERVER], VendorCredentialState::Absent, true);
    let reply = complete_with(&minted_for("srv-a"), &ports, approve_form()).await;
    check_error(
        &error_of(reply),
        400,
        InvalidRequest,
        "authorize the requested MCP server before finishing",
    );
}

#[tokio::test]
async fn a_connected_scoped_flow_completes() {
    let ports = Ports::new(&[OAUTH_SERVER], VendorCredentialState::Present, true);
    let reply = complete_with(&minted_for("srv-a"), &ports, approve_form()).await;
    assert!(matches!(reply, GatewayReply::Redirect { .. }));
}

#[tokio::test]
async fn an_m2m_scoped_flow_completes_without_a_vendor_lookup() {
    let ports = Ports::new(&[M2M_SERVER], VendorCredentialState::Unavailable, true);
    let reply = complete_with(&minted_for("srv-b"), &ports, approve_form()).await;
    assert!(matches!(reply, GatewayReply::Redirect { .. }));
}

#[tokio::test]
async fn an_unreachable_scoped_flow_is_stale() {
    let ports = Ports::new(&[OAUTH_SERVER], VendorCredentialState::Absent, false);
    let reply = complete_with(&minted_for("srv-a"), &ports, approve_form()).await;
    check_error(
        &error_of(reply),
        400,
        InvalidRequest,
        "the requested MCP server is no longer available",
    );
}

#[tokio::test]
async fn a_completed_flow_cannot_be_completed_twice() {
    let minted_flow = minted();
    let request_view = sealed_view(&minted_flow, "handle-1");
    let ports = Ports::new(&[], VendorCredentialState::Absent, true);
    let flow_guard = guard();
    let tokens = ScriptedTokens::new(&["code-jti-1"]);
    let request = complete_request(&request_view, Some("approve"), None, None);
    let first = complete_connect_flow(
        &request,
        &ports.ports(),
        &cipher(SALT),
        &flow_guard,
        &tokens,
    )
    .await;
    assert!(matches!(first, GatewayReply::Redirect { .. }));
    let replay_request = complete_request(&request_view, Some("approve"), None, None);
    let replayed = complete_connect_flow(
        &replay_request,
        &ports.ports(),
        &cipher(SALT),
        &flow_guard,
        &tokens,
    )
    .await;
    check_error(
        &error_of(replayed),
        400,
        InvalidRequest,
        "this connect flow was already completed; restart the connection",
    );
}

#[derive(Clone, Default)]
struct Recorder {
    calls: Arc<Mutex<Vec<(String, i64)>>>,
}

impl ClaimBackend for Recorder {
    fn increment(
        &self,
        key: &str,
        ttl_seconds: i64,
    ) -> impl Future<Output = Result<u64, ClaimBackendError>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), ttl_seconds));
        ready(Ok(1))
    }

    fn read(
        &self,
        _key: &str,
    ) -> impl Future<Output = Result<Option<u64>, ClaimBackendError>> + Send {
        ready(Ok(None))
    }
}

#[tokio::test]
async fn the_claim_records_the_flow_key_and_its_ttl() {
    let recorder = Recorder::default();
    let claims = Arc::clone(&recorder.calls);
    let flow_guard = SingleUseGuard::from(ClaimAuthority::Shared(recorder));
    let request_view = sealed_view(&minted(), "handle-1");
    let ports = Ports::new(&[], VendorCredentialState::Absent, true);
    let tokens = ScriptedTokens::new(&["code-jti-1"]);
    let request = complete_request(&request_view, Some("approve"), None, None);
    let reply = complete_connect_flow(
        &request,
        &ports.ports(),
        &cipher(SALT),
        &flow_guard,
        &tokens,
    )
    .await;
    assert!(matches!(reply, GatewayReply::Redirect { .. }));
    let calls = claims.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(
        calls.as_slice(),
        &[(
            format!("{FLOW_CLAIM_KEY_PREFIX}jti-1"),
            CONNECT_FLOW_TTL_SECONDS + CLAIM_TTL_BUFFER_SECONDS
        )]
    );
}

#[tokio::test]
async fn an_unavailable_claim_backend_refuses_to_complete() {
    let flow_guard = SingleUseGuard::from(ClaimAuthority::Shared(NoBackend));
    let request_view = sealed_view(&minted(), "handle-1");
    let ports = Ports::new(&[], VendorCredentialState::Absent, true);
    let tokens = ScriptedTokens::new(&["code-jti-1"]);
    let request = complete_request(&request_view, Some("approve"), None, None);
    let reply = complete_connect_flow(
        &request,
        &ports.ports(),
        &cipher(SALT),
        &flow_guard,
        &tokens,
    )
    .await;
    check_error(
        &error_of(reply),
        503,
        OauthErrorCode::TemporarilyUnavailable,
        CLAIM_UNAVAILABLE_DESCRIPTION,
    );
}

#[test]
fn the_manual_page_escapes_the_callback_url() {
    let page = manual_delivery_page("http://127.0.0.1:8080/cb?a=1&b=<x>\"'");
    assert!(page.contains("http://127.0.0.1:8080/cb?a=1&amp;b=&lt;x&gt;&quot;&#x27;"));
}
