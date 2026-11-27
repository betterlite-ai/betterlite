#![cfg(test)]

use rstest::rstest;

use super::super::*;
use crate::gateway_authorize::reply::{GatewayReply, RequestView};
use crate::gateway_authorize::tests::support::{
    BASE, CALLBACK, Page, SALT, ScriptedTokens, Teams, cipher, client_id, view,
};
use crate::gateway_dcr::contracts::{
    CONNECT_FLOW_DEBUG_KEY, DB_FAULTED_DESCRIPTION, DB_UNAVAILABLE_DESCRIPTION, PROXY_API_AUDIENCE,
};
use crate::gateway_dcr::models::ConnectFlow;
use crate::gateway_dcr::oauth_error::OauthErrorCode;
use crate::gateway_dcr::ports::{ConsentTeam, ReloadUserFailure};
use crate::gateway_dcr::sealed::open_sealed;

const HOSTED: &str = "https://claude.ai/api/mcp/auth_callback";

const fn request<'request>(
    client: &'request str,
    redirect: &'request str,
) -> AuthorizeRequest<'request> {
    AuthorizeRequest {
        client_id: client,
        redirect_uri: redirect,
        state: "st-1",
        code_challenge: Some("challenge-1"),
        code_challenge_method: Some("S256"),
        response_type: Some("code"),
    }
}

const fn call<'request>(
    authorize_request: &'request AuthorizeRequest<'request>,
    request_view: &'request RequestView,
    user: Option<&'request str>,
) -> NativeClientAuthorize<'request> {
    NativeClientAuthorize {
        view: request_view,
        request: authorize_request,
        session_user_id: user,
        now: 1_000,
    }
}

async fn native_page(teams: &Teams) -> (GatewayReply, DbCipher) {
    let client = client_id(&[CALLBACK]);
    let authorize_request = request(&client, CALLBACK);
    let request_view = view("/authorize", None, &[]);
    let db_cipher = cipher(SALT);
    let tokens = ScriptedTokens::new(&["handle-1", "jti-1"]);
    let reply = native_client_authorize(
        &call(&authorize_request, &request_view, Some("alice")),
        teams,
        &Page,
        &db_cipher,
        &tokens,
    )
    .await;
    (reply, db_cipher)
}

async fn consent_failure(failure: ReloadUserFailure, db_cipher: &DbCipher) -> GatewayReply {
    let client = client_id(&[CALLBACK]);
    let authorize_request = request(&client, CALLBACK);
    let request_view = view("/authorize", None, &[]);
    native_client_authorize(
        &call(&authorize_request, &request_view, Some("alice")),
        &Teams(Err(failure)),
        &Page,
        db_cipher,
        &ScriptedTokens::new(&[]),
    )
    .await
}

#[tokio::test]
async fn a_hosted_callback_is_refused_for_a_proxy_grant() {
    let client = client_id(&[CALLBACK, HOSTED]);
    let teams = Teams(Ok(Vec::new()));
    let authorize_request = request(&client, HOSTED);
    let request_view = view("/authorize", None, &[]);
    let reply = native_client_authorize(
        &call(&authorize_request, &request_view, Some("alice")),
        &teams,
        &Page,
        &cipher(SALT),
        &ScriptedTokens::new(&[]),
    )
    .await;
    let GatewayReply::Error(error) = reply else {
        panic!("expected an error");
    };
    assert_eq!(error.status, 400);
    assert_eq!(error.error, OauthErrorCode::InvalidRequest);
    assert_eq!(
        error.description,
        "a proxy-API grant may only redirect to a loopback address"
    );
}

#[tokio::test]
async fn a_signed_out_native_request_goes_to_the_login_page() {
    let client = client_id(&[CALLBACK]);
    let teams = Teams(Ok(Vec::new()));
    let authorize_request = request(&client, CALLBACK);
    let request_view = view("/authorize", Some("x=1"), &[]);
    let reply = native_client_authorize(
        &call(&authorize_request, &request_view, None),
        &teams,
        &Page,
        &cipher(SALT),
        &ScriptedTokens::new(&[]),
    )
    .await;
    let GatewayReply::Redirect { location, .. } = reply else {
        panic!("expected a redirect");
    };
    assert_eq!(
        location,
        format!("{BASE}/sso/key/generate?return_to=%2Fauthorize%3Fx%3D1")
    );
}

#[rstest]
#[case::unavailable(
    ReloadUserFailure::Unavailable,
    503,
    OauthErrorCode::TemporarilyUnavailable,
    DB_UNAVAILABLE_DESCRIPTION
)]
#[case::faulted(
    ReloadUserFailure::Faulted,
    503,
    OauthErrorCode::TemporarilyUnavailable,
    DB_FAULTED_DESCRIPTION
)]
#[case::unresolvable(
    ReloadUserFailure::Unresolvable,
    500,
    OauthErrorCode::ServerError,
    "the gateway is not configured to resolve users"
)]
#[case::no_active_key(
    ReloadUserFailure::NoActiveKey,
    403,
    OauthErrorCode::AccessDenied,
    "the signed-in user is not active"
)]
#[tokio::test]
async fn a_consent_lookup_failure_maps_to_its_error(
    #[case] failure: ReloadUserFailure,
    #[case] status: u16,
    #[case] code: OauthErrorCode,
    #[case] description: &str,
) {
    let db_cipher = cipher(SALT);
    let reply = consent_failure(failure, &db_cipher).await;
    let GatewayReply::Error(error) = reply else {
        panic!("expected an error");
    };
    assert_eq!(error.status, status);
    assert_eq!(error.error, code);
    assert_eq!(error.description, description);
}

#[tokio::test]
async fn the_consent_page_carries_team_labels() {
    let teams = Teams(Ok(vec![
        ConsentTeam {
            team_id: "team-1".to_owned(),
            team_alias: Some("Alpha".to_owned()),
        },
        ConsentTeam {
            team_id: "team-2".to_owned(),
            team_alias: Some(String::new()),
        },
    ]));
    let (reply, _db_cipher) = native_page(&teams).await;
    let GatewayReply::Html {
        body,
        headers,
        set_cookie,
        delete_cookie,
    } = reply
    else {
        panic!("expected a consent page");
    };
    assert_eq!(
        body,
        r#"http://127.0.0.1:8080|alice|[("team-1", "Alpha"), ("team-2", "team-2")]|handle-1|https://proxy.test:8443/authorize/complete"#
    );
    assert!(delete_cookie.is_none());
    assert!(headers.contains(&("X-Frame-Options".to_owned(), "DENY".to_owned())));
    assert!(headers.contains(&(
        "Content-Security-Policy".to_owned(),
        "frame-ancestors 'none'".to_owned()
    )));
    assert!(headers.contains(&("Cache-Control".to_owned(), "no-store".to_owned())));
    let cookie = set_cookie.expect("flow cookie");
    assert_eq!(cookie.name, "mcp_connect_flow_handle-1");
}

#[tokio::test]
async fn the_native_flow_is_bound_to_the_proxy_audience() {
    let (reply, db_cipher) = native_page(&Teams(Ok(Vec::new()))).await;
    let GatewayReply::Html { set_cookie, .. } = reply else {
        panic!("expected a consent page");
    };
    let cookie = set_cookie.expect("flow cookie");
    let opened = open_sealed::<ConnectFlow>(&cookie.value, "", &db_cipher, CONNECT_FLOW_DEBUG_KEY)
        .expect("sealed flow");
    assert!(opened.resource_server_id.is_none());
    assert_eq!(opened.audience, Some(PROXY_API_AUDIENCE));
    assert_eq!(opened.exp, 1_600);
}
