use std::collections::BTreeMap;

use rstest::rstest;
use serde_json::Number;

use crate::mcp_stored_contracts::JsonValue;
use crate::mcp_user_credentials::types::OAuthCredentialPayload;
use crate::oauth_token_resolution::ports::TokenResponse;
use crate::oauth_token_resolution::refresh_user_oauth_token;

use super::fakes::{
    BindingCall, FakeBinding, FakeBuilder, FakePost, FakeServer, Fixture, token_request,
};
use super::support::{
    FakeTable, block_on, byok_row, expires_at, exposed, now, payload, text_response,
    token_response, warnings_during,
};

const REFRESH_URL: &str = "https://auth.example/token";

fn stale_credential() -> OAuthCredentialPayload {
    payload(
        r#"{"type":"oauth2","access_token":"old-token","refresh_token":"ref-1","expires_at":"2024-01-02T03:00:00+00:00","scopes":["read"]}"#,
    )
}

fn refresh_with(
    fixture: &Fixture,
    server: &FakeServer,
    cred: &OAuthCredentialPayload,
) -> Option<OAuthCredentialPayload> {
    block_on(refresh_user_oauth_token(
        &fixture.services(),
        server,
        "u1",
        cred,
        now(),
    ))
    .expect("the refresh reports no error")
}

fn refreshed_with_body(body: TokenResponse) -> Option<OAuthCredentialPayload> {
    let mut fixture = Fixture::new();
    fixture.post = FakePost::returning(body);
    let server = FakeServer::new("s1").with_token_url(REFRESH_URL);
    refresh_with(&fixture, &server, &stale_credential())
}

fn refreshed() -> (Fixture, OAuthCredentialPayload) {
    let mut fixture = Fixture::new();
    fixture.builder = FakeBuilder::returning(token_request(
        &[("X-Client", "client-1")],
        &[("client_id", "client-1")],
    ));
    fixture.post = FakePost::returning(text_response(&[
        ("access_token", "new-token"),
        ("refresh_token", "ref-2"),
        ("expires_in", "3600"),
        ("scope", "read write"),
    ]));
    fixture.binding = FakeBinding::default().with_exchange_proof("proof-1");
    let server = FakeServer::new("s1").with_token_url(REFRESH_URL);
    let stored = refresh_with(&fixture, &server, &stale_credential())
        .expect("the refresh stores a credential");
    (fixture, stored)
}

#[test]
fn refresh_posts_the_grant_with_the_builder_headers_and_body() {
    let (fixture, _) = refreshed();
    let calls = fixture.post.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].url, REFRESH_URL);
    assert_eq!(
        calls[0].headers,
        BTreeMap::from([
            ("Accept".to_owned(), "application/json".to_owned()),
            ("X-Client".to_owned(), "client-1".to_owned()),
        ])
    );
    assert_eq!(
        calls[0].form,
        BTreeMap::from([
            ("grant_type".to_owned(), "refresh_token".to_owned()),
            ("refresh_token".to_owned(), "ref-1".to_owned()),
            ("client_id".to_owned(), "client-1".to_owned()),
        ])
    );
}

#[test]
fn refresh_stores_the_rotated_credential_and_reads_it_back() {
    let (fixture, stored) = refreshed();
    assert_eq!(exposed(stored.access_token.as_ref()), Some("new-token"));
    assert_eq!(exposed(stored.refresh_token.as_ref()), Some("ref-2"));
    assert_eq!(
        stored.expires_at.as_deref(),
        Some(expires_at(3600).as_str())
    );
    assert_eq!(
        stored.scopes,
        Some(vec!["read".to_owned(), "write".to_owned()])
    );
    assert_eq!(stored.identity_binding_proof.as_deref(), Some("proof-1"));
    assert_eq!(fixture.table.stored().len(), 1);
    assert_eq!(
        fixture.table.reads(),
        vec![("u1".to_owned(), "s1".to_owned())]
    );
}

#[test]
fn refresh_presents_the_refresh_token_and_body_to_the_binding_port() {
    let (fixture, _) = refreshed();
    let calls = fixture.binding.calls();
    assert_eq!(calls.len(), 1);
    let BindingCall::Enforce {
        server_id,
        user_id,
        grant_type,
        presented,
        body_fields,
    } = &calls[0]
    else {
        panic!("the refresh presents the token, not a stored proof");
    };
    assert_eq!(server_id, "s1");
    assert_eq!(user_id.as_deref(), Some("u1"));
    assert_eq!(grant_type, "refresh_token");
    assert_eq!(presented.as_deref(), Some("ref-1"));
    assert_eq!(
        body_fields,
        &vec![
            "access_token".to_owned(),
            "expires_in".to_owned(),
            "refresh_token".to_owned(),
            "scope".to_owned(),
        ]
    );
}

#[test]
fn refresh_stores_even_when_a_byok_row_holds_the_credential() {
    let mut fixture = Fixture::new();
    fixture.table = FakeTable::new(vec![byok_row(&fixture.cipher, "u1", "s1")]);
    let server = FakeServer::new("s1").with_token_url(REFRESH_URL);
    let stored = refresh_with(&fixture, &server, &stale_credential())
        .expect("the refresh stores a credential");
    assert_eq!(exposed(stored.access_token.as_ref()), Some("new-token"));
    assert_eq!(fixture.table.reads().len(), 1);
}

#[test]
fn refresh_returns_none_when_the_stored_proof_does_not_match() {
    let mut fixture = Fixture::new();
    fixture.binding = FakeBinding::default().disagreeing();
    let server = FakeServer::new("s1")
        .with_token_url(REFRESH_URL)
        .enforcing();
    assert!(refresh_with(&fixture, &server, &stale_credential()).is_none());
    assert!(fixture.builder.calls().is_empty());
    assert!(fixture.post.calls().is_empty());
    assert!(matches!(
        fixture.binding.calls().as_slice(),
        [BindingCall::Proof { .. }]
    ));
}

#[test]
fn refresh_returns_none_without_a_stored_refresh_token() {
    let cred = payload(r#"{"type":"oauth2","access_token":"old-token"}"#);
    let fixture = Fixture::new();
    let server = FakeServer::new("s1").with_token_url(REFRESH_URL);
    assert!(refresh_with(&fixture, &server, &cred).is_none());
    assert!(fixture.builder.calls().is_empty());
}

#[test]
fn refresh_returns_none_for_an_empty_stored_refresh_token() {
    let cred = payload(r#"{"type":"oauth2","access_token":"old-token","refresh_token":""}"#);
    let fixture = Fixture::new();
    let server = FakeServer::new("s1").with_token_url(REFRESH_URL);
    assert!(refresh_with(&fixture, &server, &cred).is_none());
    assert!(fixture.builder.calls().is_empty());
}

#[test]
fn refresh_returns_none_without_a_token_url() {
    let fixture = Fixture::new();
    let server = FakeServer::new("s1");
    assert!(refresh_with(&fixture, &server, &stale_credential()).is_none());
    assert!(fixture.builder.calls().is_empty());
}

#[test]
fn refresh_treats_an_empty_token_url_as_absent() {
    let fixture = Fixture::new();
    let server = FakeServer::new("s1").with_token_url("");
    assert!(refresh_with(&fixture, &server, &stale_credential()).is_none());
    assert!(fixture.builder.calls().is_empty());
}

#[test]
fn refresh_returns_none_when_the_request_builder_fails() {
    let mut fixture = Fixture::new();
    fixture.builder = FakeBuilder::failing();
    let server = FakeServer::new("s1").with_token_url(REFRESH_URL);
    let warnings = warnings_during(|| {
        assert!(refresh_with(&fixture, &server, &stale_credential()).is_none());
    });
    assert!(fixture.post.calls().is_empty());
    assert!(fixture.table.stored().is_empty());
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("refresh_user_oauth_token: refresh request failed"));
}

#[test]
fn refresh_returns_none_when_the_token_endpoint_fails() {
    let mut fixture = Fixture::new();
    fixture.post = FakePost::failing();
    let server = FakeServer::new("s1").with_token_url(REFRESH_URL);
    let warnings = warnings_during(|| {
        assert!(refresh_with(&fixture, &server, &stale_credential()).is_none());
    });
    assert!(fixture.table.stored().is_empty());
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("refresh_user_oauth_token: refresh request failed"));
}

#[test]
fn refresh_returns_none_when_binding_denies_the_exchange() {
    let mut fixture = Fixture::new();
    fixture.binding = FakeBinding::denying();
    let server = FakeServer::new("s1").with_token_url(REFRESH_URL);
    assert!(refresh_with(&fixture, &server, &stale_credential()).is_none());
    assert!(fixture.table.stored().is_empty());
    assert!(matches!(
        fixture.binding.calls().as_slice(),
        [BindingCall::Enforce { .. }]
    ));
}

#[test]
fn refresh_returns_none_when_the_response_has_no_access_token() {
    let mut fixture = Fixture::new();
    fixture.post = FakePost::returning(text_response(&[("refresh_token", "ref-2")]));
    let server = FakeServer::new("s1").with_token_url(REFRESH_URL);
    let warnings = warnings_during(|| {
        assert!(refresh_with(&fixture, &server, &stale_credential()).is_none());
    });
    assert!(fixture.table.stored().is_empty());
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("refresh_user_oauth_token: token response missing access_token"));
}

#[test]
fn refresh_treats_an_empty_access_token_as_missing() {
    let body = text_response(&[("access_token", "")]);
    assert!(refreshed_with_body(body).is_none());
}

#[test]
fn refresh_treats_a_non_text_access_token_as_missing() {
    let body = token_response(&[("access_token", JsonValue::Number(Number::from(5)))]);
    assert!(refreshed_with_body(body).is_none());
}

fn with_expiry(value: JsonValue) -> TokenResponse {
    token_response(&[
        ("access_token", JsonValue::String("new-token".to_owned())),
        ("expires_in", value),
    ])
}

#[rstest]
#[case::omitted(text_response(&[("access_token", "new-token")]), Some("ref-1"))]
#[case::empty(text_response(&[("access_token", "new-token"), ("refresh_token", "")]), Some("ref-1"))]
#[case::rotated(text_response(&[("access_token", "new-token"), ("refresh_token", "ref-2")]), Some("ref-2"))]
fn refresh_keeps_or_rotates_the_refresh_token(
    #[case] body: TokenResponse,
    #[case] expected: Option<&str>,
) {
    let stored = refreshed_with_body(body).expect("the refresh stores a credential");
    assert_eq!(exposed(stored.refresh_token.as_ref()), expected);
}

#[rstest]
#[case::omitted(text_response(&[("access_token", "new-token")]), vec!["read".to_owned()])]
#[case::blank(text_response(&[("access_token", "new-token"), ("scope", "   ")]), vec!["read".to_owned()])]
#[case::granted(text_response(&[("access_token", "new-token"), ("scope", "admin write")]), vec!["admin".to_owned(), "write".to_owned()])]
fn refresh_keeps_or_replaces_the_granted_scopes(
    #[case] body: TokenResponse,
    #[case] expected: Vec<String>,
) {
    let stored = refreshed_with_body(body).expect("the refresh stores a credential");
    assert_eq!(stored.scopes, Some(expected));
}

#[test]
fn refresh_stores_a_token_without_an_expiry_when_expires_in_is_absent() {
    let body = text_response(&[("access_token", "new-token")]);
    let stored = refreshed_with_body(body).expect("the refresh stores a credential");
    assert_eq!(stored.expires_at, None);
}

#[rstest]
#[case::number(
    with_expiry(JsonValue::Number(Number::from(3600))),
    Some(expires_at(3600))
)]
#[case::float(
    with_expiry(JsonValue::Number(
        Number::from_f64(59.9).expect("the test float is finite")
    )),
    Some(expires_at(59))
)]
#[case::numeric_string(
    text_response(&[("access_token", "new-token"), ("expires_in", " 600 ")]),
    Some(expires_at(600))
)]
#[case::negative_number(
    with_expiry(JsonValue::Number(Number::from(-10))),
    Some(expires_at(-10))
)]
#[case::fractional_string(
    text_response(&[("access_token", "new-token"), ("expires_in", "1.5")]),
    None
)]
#[case::text(
    text_response(&[("access_token", "new-token"), ("expires_in", "soon")]),
    None
)]
#[case::boolean(with_expiry(JsonValue::Bool(true)), None)]
#[case::null(with_expiry(JsonValue::Null), None)]
fn refresh_reads_expires_in_like_int(
    #[case] body: TokenResponse,
    #[case] expected: Option<String>,
) {
    let stored = refreshed_with_body(body).expect("the refresh stores a credential");
    assert_eq!(stored.expires_at, expected);
}
