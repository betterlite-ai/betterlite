use std::sync::Mutex;

use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;

use crate::xai_login::error::XaiLoginError;
use crate::xai_login::service::{
    Existing, LoginOptions, StoredRecord, TerminalOutput, UserOutput as _, classify,
};
use crate::xai_tokens::codec::decode_xai_oauth;
use crate::xai_tokens::error::XaiOauthError;
use crate::xai_tokens::rules::CLIENT_ID;

use super::support::{
    BrokenWriter, DISCOVERY_REPLY, FRESH, Fixture, NOW, Reply, STALE, STALE_WITHOUT_REFRESH,
    TOKEN_REPLY, callback_when, port_of,
};

const VERIFIER: &str = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc";
const ONLY_REFRESH: &str = r#"{"refresh_token":"only","token_type":"Bearer","token_endpoint":"https://auth.x.ai/oauth2/token","expires_at":5000}"#;
const BLANK_ACCESS: &str = r#"{"access_token":"","refresh_token":"r","token_type":"Bearer","token_endpoint":"https://auth.x.ai/oauth2/token","expires_at":5000}"#;
const UNREADABLE_EXPIRY: &str = r#"{"access_token":"a","refresh_token":"r","token_type":"Bearer","token_endpoint":"https://auth.x.ai/oauth2/token","expires_at":"later"}"#;
const FRESH_BLANK_REFRESH: &str = r#"{"access_token":"live-access","refresh_token":"","id_token":"live-id","token_type":"Bearer","token_endpoint":"https://auth.x.ai/oauth2/token","expires_at":5000}"#;
const EXPIRED_BLANK_REFRESH: &str = r#"{"access_token":"a","refresh_token":"","token_type":"Bearer","token_endpoint":"https://auth.x.ai/oauth2/token","expires_at":1000}"#;

fn options() -> LoginOptions {
    LoginOptions {
        force: false,
        no_browser: false,
    }
}

fn stored(json: &str) -> StoredRecord {
    decode_xai_oauth(json.as_bytes()).unwrap()
}

#[tokio::test]
async fn an_unexpired_record_is_returned_without_a_request() {
    let fixture = Fixture::new(Some(FRESH), Vec::new());
    let record = fixture.service.login(options()).await.unwrap();
    assert_eq!(record.access_token.expose_secret(), "live-access");
    assert_eq!(record.refresh_token.expose_secret(), "live-refresh");
    assert_eq!(record.id_token.unwrap().expose_secret(), "live-id");
    assert_eq!(record.token_type, "Bearer");
    assert_eq!(record.token_endpoint, "https://auth.x.ai/oauth2/token");
    assert_eq!(record.expires_at, 5_000);
    assert!(fixture.seen().is_empty());
    assert_eq!(fixture.service.browser.opens(), 0);
}

#[tokio::test]
async fn force_ignores_the_existing_record() {
    let fixture = Fixture::new(Some(FRESH), vec![Reply::Fail]);
    let error = fixture
        .service
        .login(LoginOptions {
            force: true,
            no_browser: false,
        })
        .await
        .unwrap_err();
    assert!(matches!(error, XaiLoginError::Oauth(_)));
    let seen = fixture.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "GET");
    assert_eq!(
        seen[0].url,
        "https://auth.x.ai/.well-known/openid-configuration"
    );
}

#[tokio::test]
async fn an_expired_record_is_refreshed_before_authorizing() {
    let fixture = Fixture::new(Some(STALE), vec![Reply::Body(200, TOKEN_REPLY)]);
    let record = fixture.service.login(options()).await.unwrap();
    assert_eq!(record.access_token.expose_secret(), "new-access");
    let seen = fixture.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, "https://auth.x.ai/oauth2/token");
    assert!(seen[0].body.contains("grant_type=refresh_token"));
    assert!(seen[0].body.contains("refresh_token=old-refresh"));
    assert!(fixture.stored_file().contains("new-access"));
    assert_eq!(fixture.service.browser.opens(), 0);
}

#[tokio::test]
async fn a_failed_refresh_falls_through_to_authorization() {
    let fixture = Fixture::new(
        Some(STALE),
        vec![Reply::Body(400, "{}"), Reply::Body(500, "{}")],
    );
    let error = fixture.service.login(options()).await.unwrap_err();
    assert!(matches!(error, XaiLoginError::Oauth(_)));
    let seen = fixture.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[1].method, "GET");
    assert_eq!(
        seen[1].url,
        "https://auth.x.ai/.well-known/openid-configuration"
    );
}

#[tokio::test]
async fn a_fresh_record_with_a_blank_refresh_token_is_returned() {
    let fixture = Fixture::new(Some(FRESH_BLANK_REFRESH), Vec::new());
    let record = fixture.service.login(options()).await.unwrap();
    assert_eq!(record.access_token.expose_secret(), "live-access");
    assert_eq!(record.refresh_token.expose_secret(), "");
    assert!(fixture.seen().is_empty());
    assert_eq!(fixture.service.browser.opens(), 0);
}

#[tokio::test]
async fn a_transport_failure_on_refresh_ends_the_login() {
    let fixture = Fixture::new(Some(STALE), vec![Reply::Fail]);
    let error = fixture.service.login(options()).await.unwrap_err();
    assert!(matches!(
        error,
        XaiLoginError::Oauth(XaiOauthError::Transport { stage: "token", .. })
    ));
    let seen = fixture.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(fixture.service.browser.opens(), 0);
}

#[tokio::test]
async fn a_record_without_a_refresh_token_authorizes() {
    let fixture = Fixture::new(Some(STALE_WITHOUT_REFRESH), vec![Reply::Fail]);
    let error = fixture.service.login(options()).await.unwrap_err();
    assert!(matches!(error, XaiLoginError::Oauth(_)));
    let seen = fixture.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "GET");
}

#[tokio::test]
async fn a_record_without_an_access_token_authorizes() {
    let fixture = Fixture::new(Some(ONLY_REFRESH), vec![Reply::Fail]);
    let error = fixture.service.login(options()).await.unwrap_err();
    assert!(matches!(error, XaiLoginError::Oauth(_)));
    let seen = fixture.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "GET");
}

#[tokio::test]
async fn a_matching_callback_exchanges_the_code_and_stores_the_record() {
    let fixture = Fixture::new(
        None,
        vec![
            Reply::Body(200, DISCOVERY_REPLY),
            Reply::Body(200, TOKEN_REPLY),
        ],
    );
    let service = &fixture.service;
    let (login, response) = tokio::join!(
        service.login(options()),
        callback_when(
            || service.browser.url(),
            "/callback?code=code-1&state=state-test"
        ),
    );
    let record = login.unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(record.access_token.expose_secret(), "new-access");
    assert_eq!(record.refresh_token.expose_secret(), "new-refresh");
    assert_eq!(record.id_token.unwrap().expose_secret(), "new-id");
    assert_eq!(record.expires_at, NOW + 7_200);
    let port = port_of(&service.browser.url().unwrap());
    let seen = fixture.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(
        seen[0].url,
        "https://auth.x.ai/.well-known/openid-configuration"
    );
    let exchange = &seen[1];
    assert_eq!(exchange.method, "POST");
    assert_eq!(exchange.url, "https://auth.x.ai/oauth2/token");
    assert_eq!(exchange.content_type, "application/x-www-form-urlencoded");
    assert_eq!(
        exchange.body,
        format!(
            "grant_type=authorization_code&code=code-1&redirect_uri=http%3A%2F%2F127.0.0.1%3A{port}%2Fcallback&client_id={CLIENT_ID}&code_verifier={VERIFIER}"
        )
    );
    assert!(fixture.stored_file().contains("new-access"));
}

#[tokio::test]
async fn the_authorization_url_reaches_the_browser() {
    let fixture = Fixture::new(
        None,
        vec![
            Reply::Body(200, DISCOVERY_REPLY),
            Reply::Body(200, TOKEN_REPLY),
        ],
    );
    let service = &fixture.service;
    let (_record, _response) = tokio::join!(
        service.login(options()),
        callback_when(
            || service.browser.url(),
            "/callback?code=code-1&state=state-test"
        ),
    );
    let url = service.browser.url().unwrap();
    let port = port_of(&url);
    assert_eq!(service.browser.opens(), 1);
    assert_eq!(
        url,
        format!(
            "https://auth.x.ai/oauth2/authorize?response_type=code&client_id={CLIENT_ID}&redirect_uri=http%3A%2F%2F127.0.0.1%3A{port}%2Fcallback&scope=openid+profile+email+offline_access+grok-cli%3Aaccess+api%3Aaccess&code_challenge=3Ev4DHdHPRMPoN6GukAY_pi7IUAF5qWJHRK6kURvnoE&code_challenge_method=S256&state=state-test&nonce=nonce-test"
        )
    );
    assert!(fixture.service.output.text().is_empty());
}

#[tokio::test]
async fn no_browser_prints_the_url_without_opening_it() {
    let fixture = Fixture::new(
        None,
        vec![
            Reply::Body(200, DISCOVERY_REPLY),
            Reply::Body(200, TOKEN_REPLY),
        ],
    );
    let service = &fixture.service;
    let (record, response) = tokio::join!(
        service.login(LoginOptions {
            force: false,
            no_browser: true,
        }),
        callback_when(
            || service.output.text().lines().nth(1).map(str::to_owned),
            "/callback?code=code-1&state=state-test",
        ),
    );
    record.unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(service.browser.opens(), 0);
    let text = service.output.text();
    assert!(text.starts_with("Open this URL to authenticate with xAI:\n"));
    assert!(text.contains("https://auth.x.ai/oauth2/authorize?response_type=code"));
    assert!(text.ends_with('\n'));
}

#[tokio::test]
async fn a_refused_browser_prints_the_url() {
    let mut fixture = Fixture::new(
        None,
        vec![
            Reply::Body(200, DISCOVERY_REPLY),
            Reply::Body(200, TOKEN_REPLY),
        ],
    );
    fixture.service.browser.result = false;
    let service = &fixture.service;
    let (record, _response) = tokio::join!(
        service.login(options()),
        callback_when(
            || service.output.text().lines().nth(1).map(str::to_owned),
            "/callback?code=code-1&state=state-test",
        ),
    );
    record.unwrap();
    assert_eq!(service.browser.opens(), 1);
    assert!(service.output.text().contains("https://auth.x.ai"));
}

#[tokio::test]
async fn a_state_mismatch_ends_the_login() {
    let fixture = Fixture::new(None, vec![Reply::Body(200, DISCOVERY_REPLY)]);
    let service = &fixture.service;
    let (result, response) = tokio::join!(
        service.login(options()),
        callback_when(
            || service.browser.url(),
            "/callback?code=code-1&state=state-other"
        ),
    );
    assert!(matches!(result, Err(XaiLoginError::StateMismatch)));
    assert_eq!(response.status, 400);
    assert_eq!(
        response.content_type.as_deref(),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(fixture.seen().len(), 1);
}

#[tokio::test]
async fn a_provider_error_ends_the_login_with_its_description() {
    let fixture = Fixture::new(None, vec![Reply::Body(200, DISCOVERY_REPLY)]);
    let service = &fixture.service;
    let (result, response) = tokio::join!(
        service.login(options()),
        callback_when(
            || service.browser.url(),
            "/callback?error=access_denied&error_description=user+denied&state=state-test",
        ),
    );
    assert_eq!(response.status, 200);
    assert!(matches!(
        result,
        Err(XaiLoginError::Provider { description }) if description == "user denied"
    ));
    assert_eq!(fixture.seen().len(), 1);
}

#[tokio::test]
async fn a_missing_code_ends_the_login() {
    let fixture = Fixture::new(None, vec![Reply::Body(200, DISCOVERY_REPLY)]);
    let service = &fixture.service;
    let (result, response) = tokio::join!(
        service.login(options()),
        callback_when(|| service.browser.url(), "/callback?state=state-test"),
    );
    assert_eq!(response.status, 200);
    assert!(matches!(result, Err(XaiLoginError::MissingCode)));
    assert_eq!(fixture.seen().len(), 1);
}

#[tokio::test]
async fn an_entropy_failure_stops_the_login() {
    let mut fixture = Fixture::new(None, vec![Reply::Body(200, DISCOVERY_REPLY)]);
    fixture.service.entropy.fail = true;
    let error = fixture.service.login(options()).await.unwrap_err();
    assert!(matches!(error, XaiLoginError::Entropy { .. }));
    assert_eq!(fixture.service.browser.opens(), 0);
}

#[tokio::test]
async fn an_output_failure_stops_the_login() {
    let mut fixture = Fixture::new(None, vec![Reply::Body(200, DISCOVERY_REPLY)]);
    fixture.service.output.fail = true;
    let error = fixture
        .service
        .login(LoginOptions {
            force: false,
            no_browser: true,
        })
        .await
        .unwrap_err();
    assert!(matches!(error, XaiLoginError::Output { .. }));
}

#[tokio::test]
async fn an_overflowing_deadline_stops_the_login() {
    let mut fixture = Fixture::new(None, vec![Reply::Body(200, DISCOVERY_REPLY)]);
    fixture.service.oauth.clock = super::support::FixedClock(Decimal::MAX);
    let error = fixture
        .service
        .login(LoginOptions {
            force: false,
            no_browser: true,
        })
        .await
        .unwrap_err();
    assert!(matches!(error, XaiLoginError::Deadline));
}

#[rstest]
#[case::fresh(FRESH, true)]
#[case::expired(STALE, false)]
fn classifies_an_existing_record(#[case] json: &str, #[case] fresh: bool) {
    let existing = classify(stored(json), Decimal::from(NOW));
    let matched = if fresh {
        matches!(existing, Existing::Fresh(_))
    } else {
        matches!(existing, Existing::Refreshable(_))
    };
    assert!(matched);
}

#[rstest]
#[case::only_a_refresh_token(ONLY_REFRESH)]
#[case::blank_access_token(BLANK_ACCESS)]
#[case::expired_blank_refresh_token(EXPIRED_BLANK_REFRESH)]
fn a_record_the_declared_type_cannot_hold_is_unusable(#[case] json: &str) {
    assert!(matches!(
        classify(stored(json), Decimal::from(NOW)),
        Existing::Unusable
    ));
}

#[test]
fn a_fresh_record_with_a_blank_refresh_token_is_fresh() {
    let Existing::Fresh(record) = classify(stored(FRESH_BLANK_REFRESH), Decimal::from(NOW)) else {
        panic!("a fresh record was not returned");
    };
    assert_eq!(record.refresh_token.expose_secret(), "");
}

#[test]
fn a_fresh_record_keeps_every_stored_field() {
    let Existing::Fresh(record) = classify(stored(FRESH), Decimal::from(NOW)) else {
        panic!("a fresh record was not returned");
    };
    assert_eq!(record.id_token.unwrap().expose_secret(), "live-id");
    assert_eq!(record.token_type, "Bearer");
    assert_eq!(record.token_endpoint, "https://auth.x.ai/oauth2/token");
    assert_eq!(record.expires_at, 5_000);
}

#[test]
fn terminal_output_writes_the_text() {
    let output = TerminalOutput {
        writer: Mutex::new(Vec::new()),
    };
    output.write("Open this URL\n").unwrap();
    assert_eq!(
        output.writer.lock().unwrap().as_slice(),
        b"Open this URL\n".as_slice()
    );
}

#[test]
fn terminal_output_reports_a_closed_writer() {
    let output = TerminalOutput {
        writer: Mutex::new(BrokenWriter),
    };
    assert!(output.write("x").is_err());
}

#[test]
fn a_record_with_an_unreadable_expiry_is_refreshable() {
    assert!(matches!(
        classify(stored(UNREADABLE_EXPIRY), Decimal::from(NOW)),
        Existing::Refreshable(_)
    ));
}
