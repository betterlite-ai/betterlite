#![cfg(test)]

mod native;

use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use rstest::rstest;

use super::*;
use crate::gateway_authorize::reply::GatewayReply;
use crate::gateway_authorize::tests::support::{
    BASE, CALLBACK, OAUTH_SERVER, SALT, ScriptedTokens, cipher, client_id, flow, registry, view,
};
use crate::gateway_dcr::contracts::{
    CONNECT_FLOW_DEBUG_KEY, CONNECT_FLOW_TTL_SECONDS, MAX_STATE_LENGTH,
};
use crate::gateway_dcr::models::ConnectFlow;
use crate::gateway_dcr::oauth_error::OauthErrorCode;
use crate::gateway_dcr::sealed::open_sealed;

struct Case {
    client: String,
    redirect_uri: &'static str,
    response_type: Option<&'static str>,
    challenge: Option<&'static str>,
    method: Option<&'static str>,
    state: String,
}

impl Case {
    fn valid() -> Self {
        Self {
            client: client_id(&[CALLBACK]),
            redirect_uri: CALLBACK,
            response_type: Some("code"),
            challenge: Some("challenge-1"),
            method: Some("S256"),
            state: "st-1".to_owned(),
        }
    }

    const fn request(&self) -> AuthorizeRequest<'_> {
        AuthorizeRequest {
            client_id: &self.client,
            redirect_uri: self.redirect_uri,
            state: &self.state,
            code_challenge: self.challenge,
            code_challenge_method: self.method,
            response_type: self.response_type,
        }
    }
}

fn signed_in_reply(
    resource: Option<&str>,
    servers: &ServerRegistry,
    tokens: &ScriptedTokens,
) -> GatewayReply {
    let case = Case::valid();
    let request = case.request();
    let request_view = view("/authorize", None, &[]);
    aggregate_authorize(
        &AggregateAuthorize {
            view: &request_view,
            request: &request,
            session_user_id: Some("alice"),
            resource,
            now: 1_000,
        },
        servers,
        &cipher(SALT),
        tokens,
    )
}

#[test]
fn an_unknown_client_is_refused_before_any_redirect() {
    let mut case = Case::valid();
    case.client = "llm_dcrc_bogus".to_owned();
    let error = rejected_authorize_request(&case.request(), &cipher(SALT)).unwrap();
    assert_eq!(error.status, 400);
    assert_eq!(error.error, OauthErrorCode::InvalidClient);
    assert_eq!(error.description, "unknown or malformed client_id");
}

#[test]
fn a_redirect_uri_must_be_registered() {
    let mut case = Case::valid();
    case.redirect_uri = "https://evil.test/cb";
    let error = rejected_authorize_request(&case.request(), &cipher(SALT)).unwrap();
    assert_eq!(error.status, 400);
    assert_eq!(error.error, OauthErrorCode::InvalidRequest);
    assert_eq!(
        error.description,
        "redirect_uri is not registered for this client"
    );
}

#[test]
fn response_type_must_be_code() {
    let mut case = Case::valid();
    case.response_type = Some("token");
    let error = rejected_authorize_request(&case.request(), &cipher(SALT)).unwrap();
    assert_eq!(error.error, OauthErrorCode::UnsupportedResponseType);
    assert_eq!(error.description, "response_type must be 'code'");
}

#[rstest]
#[case::missing_challenge(None, Some("S256"))]
#[case::empty_challenge(Some(""), Some("S256"))]
#[case::plain_method(Some("c"), Some("plain"))]
fn pkce_is_required_with_s256(
    #[case] challenge: Option<&'static str>,
    #[case] method: Option<&'static str>,
) {
    let mut case = Case::valid();
    case.challenge = challenge;
    case.method = method;
    let error = rejected_authorize_request(&case.request(), &cipher(SALT)).unwrap();
    assert_eq!(error.status, 400);
    assert_eq!(error.error, OauthErrorCode::InvalidRequest);
    assert_eq!(error.description, PKCE_DESCRIPTION);
}

#[test]
fn a_valid_pkce_request_is_accepted() {
    let db_cipher = cipher(SALT);
    assert!(rejected_authorize_request(&Case::valid().request(), &db_cipher).is_none());
}

#[test]
fn state_is_capped_at_the_maximum_length() {
    let db_cipher = cipher(SALT);
    let mut at_limit = Case::valid();
    at_limit.state = "s".repeat(MAX_STATE_LENGTH);
    assert!(rejected_authorize_request(&at_limit.request(), &db_cipher).is_none());
    let mut past = Case::valid();
    past.state = "s".repeat(MAX_STATE_LENGTH + 1);
    let error = rejected_authorize_request(&past.request(), &db_cipher).unwrap();
    assert_eq!(error.status, 400);
    assert_eq!(
        error.description,
        format!("state must be at most {MAX_STATE_LENGTH} characters")
    );
}

#[test]
fn a_signed_out_request_returns_to_the_login_page() {
    let case = Case::valid();
    let request = case.request();
    let tokens = ScriptedTokens::new(&["handle-1", "jti-1"]);
    let servers = registry(&[]);
    let request_view = view("/authorize", Some("a=1"), &[]);
    let reply = aggregate_authorize(
        &AggregateAuthorize {
            view: &request_view,
            request: &request,
            session_user_id: None,
            resource: None,
            now: 1_000,
        },
        &servers,
        &cipher(SALT),
        &tokens,
    );
    assert_eq!(
        reply,
        GatewayReply::Redirect {
            location: format!("{BASE}/sso/key/generate?return_to=%2Fauthorize%3Fa%3D1"),
            set_cookie: None,
            delete_cookie: None,
        }
    );
}

#[test]
fn an_aggregate_authorization_mints_a_flow_cookie() {
    let tokens = ScriptedTokens::new(&["handle-1", "jti-1"]);
    let servers = registry(&[]);
    let reply = signed_in_reply(None, &servers, &tokens);
    let GatewayReply::Redirect {
        location,
        set_cookie,
        delete_cookie,
    } = reply
    else {
        panic!("expected a redirect");
    };
    assert_eq!(location, format!("{BASE}/ui/connect?connect_flow=handle-1"));
    assert!(delete_cookie.is_none());
    let cookie = set_cookie.expect("flow cookie");
    assert_eq!(cookie.name, "mcp_connect_flow_handle-1");
    assert_eq!(cookie.max_age_seconds, CONNECT_FLOW_TTL_SECONDS);
    assert_eq!(cookie.path, "/");
    assert!(cookie.secure);
}

#[test]
fn the_flow_cookie_opens_as_the_connect_flow() {
    let tokens = ScriptedTokens::new(&["handle-1", "jti-1"]);
    let servers = registry(&[]);
    let reply = signed_in_reply(None, &servers, &tokens);
    let GatewayReply::Redirect { set_cookie, .. } = reply else {
        panic!("expected a redirect");
    };
    let cookie = set_cookie.expect("flow cookie");
    let opened =
        open_sealed::<ConnectFlow>(&cookie.value, "", &cipher(SALT), CONNECT_FLOW_DEBUG_KEY)
            .expect("sealed flow");
    assert_eq!(opened.user_id, "alice");
    assert_eq!(opened.jti, "jti-1");
    assert_eq!(opened.exp, 1_000 + CONNECT_FLOW_TTL_SECONDS);
    assert_eq!(opened.state, "st-1");
    assert_eq!(opened.code_challenge, "challenge-1");
    assert!(opened.audience.is_none());
    assert!(opened.resource_server_id.is_none());
}

#[test]
fn a_scoped_resource_seals_the_server_into_the_flow() {
    let tokens = ScriptedTokens::new(&["handle-1", "jti-1"]);
    let servers = registry(&[OAUTH_SERVER]);
    let resource = format!("{BASE}/srv-a/mcp");
    let reply = signed_in_reply(Some(&resource), &servers, &tokens);
    let GatewayReply::Redirect { set_cookie, .. } = reply else {
        panic!("expected a redirect");
    };
    let cookie = set_cookie.expect("flow cookie");
    let opened =
        open_sealed::<ConnectFlow>(&cookie.value, "", &cipher(SALT), CONNECT_FLOW_DEBUG_KEY)
            .expect("sealed flow");
    assert_eq!(opened.resource_server_id.as_deref(), Some("srv-a"));
}

#[test]
fn a_flow_cookie_uses_the_base_path_and_secure_flag() {
    let db_cipher = cipher(SALT);
    let secure = set_flow_cookie("h-1", &flow(), "https://proxy.test/x", &db_cipher).unwrap();
    assert_eq!(secure.name, "mcp_connect_flow_h-1");
    assert_eq!(secure.path, "/x");
    assert!(secure.secure);
    let plain = set_flow_cookie("h-1", &flow(), "http://proxy.test/tenant", &db_cipher).unwrap();
    assert_eq!(plain.path, "/tenant");
    assert!(!plain.secure);
}

#[test]
fn a_cipher_without_a_key_refuses_to_seal_the_flow() {
    let keyless = StoredCipher {
        settings: CipherSettings {
            signing_key: None,
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    };
    let error = set_flow_cookie("h-1", &flow(), BASE, &keyless).unwrap_err();
    assert_eq!(error.status, 500);
    assert_eq!(error.error, OauthErrorCode::ServerError);
    assert_eq!(error.description, CONNECT_FLOW_SEAL_DESCRIPTION);
}

#[test]
fn login_redirect_encodes_the_relative_return_to() {
    let GatewayReply::Redirect { location, .. } = login_redirect(BASE, "/authorize", None) else {
        panic!("expected a redirect");
    };
    assert_eq!(
        location,
        format!("{BASE}/sso/key/generate?return_to=%2Fauthorize")
    );
}
