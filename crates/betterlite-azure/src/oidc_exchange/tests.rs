#![cfg(test)]

use std::fs;
use std::io;
use std::path::Path;

use secrecy::ExposeSecret as _;

use crate::oidc_exchange::token::decode_azure_oidc_token;
use fixture::{
    ASSERTION, ASSERTION_ENV, ASSERTION_REF, CLIENT, Fixture, Reply, TENANT, default_input,
    form_map, input,
};

mod cases;
mod fixture;

const TOKEN_BODY: &str = r#"{"access_token":"tok-value","expires_in":3600}"#;

#[test]
fn azure_oidc_exchange_credential_identity_isolation() {
    isolation_tokens();
    isolation_urls();
    isolation_forms();
}

fn isolation_fixture() -> Fixture {
    Fixture::new(vec![
        Reply::ok(r#"{"access_token":"one","expires_in":3600}"#),
        Reply::ok(r#"{"access_token":"two","expires_in":3600}"#),
    ])
    .with_environment(&[("OTHER_ASSERTION", "other-assertion")])
}

fn isolation_recorded() -> Vec<fixture::Recorded> {
    let fixture = isolation_fixture();
    let exchange = fixture.exchange();
    exchange
        .token(
            &input(
                ASSERTION_REF,
                Some("client-one"),
                Some("tenant-one"),
                Some("scope-one"),
            ),
            &fixture.resolver(),
        )
        .unwrap();
    exchange
        .token(
            &input(
                "os.environ/OTHER_ASSERTION",
                Some("client-two"),
                Some("tenant-two"),
                Some("scope-two"),
            ),
            &fixture.resolver(),
        )
        .unwrap();
    fixture.requests()
}

fn isolation_tokens() {
    let fixture = isolation_fixture();
    let exchange = fixture.exchange();
    let first = exchange
        .token(
            &input(
                ASSERTION_REF,
                Some("client-one"),
                Some("tenant-one"),
                Some("scope-one"),
            ),
            &fixture.resolver(),
        )
        .unwrap();
    let second = exchange
        .token(
            &input(
                "os.environ/OTHER_ASSERTION",
                Some("client-two"),
                Some("tenant-two"),
                Some("scope-two"),
            ),
            &fixture.resolver(),
        )
        .unwrap();
    assert_eq!(first.expose_secret(), "one");
    assert_eq!(second.expose_secret(), "two");
    assert_eq!(fixture.requests().len(), 2);
}

fn isolation_urls() {
    let requests = isolation_recorded();
    assert_eq!(
        requests[0].url,
        "https://login.microsoftonline.com/tenant-one/oauth2/v2.0/token"
    );
    assert_eq!(
        requests[1].url,
        "https://login.microsoftonline.com/tenant-two/oauth2/v2.0/token"
    );
}

fn isolation_forms() {
    let requests = isolation_recorded();
    let first_form = form_map(&requests[0]);
    assert_eq!(
        first_form.get("client_id").map(String::as_str),
        Some("client-one")
    );
    assert_eq!(
        first_form.get("scope").map(String::as_str),
        Some("scope-one")
    );
    assert_eq!(
        first_form.get("client_assertion").map(String::as_str),
        Some(ASSERTION)
    );
    let second_form = form_map(&requests[1]);
    assert_eq!(
        second_form.get("client_id").map(String::as_str),
        Some("client-two")
    );
    assert_eq!(
        second_form.get("scope").map(String::as_str),
        Some("scope-two")
    );
    assert_eq!(
        second_form.get("client_assertion").map(String::as_str),
        Some("other-assertion")
    );
}

// the scope joins the cache identity here, so two audiences never share one token
#[test]
fn azure_oidc_exchange_scope_identity_isolation() {
    let fixture = Fixture::new(vec![
        Reply::ok(r#"{"access_token":"one","expires_in":3600}"#),
        Reply::ok(r#"{"access_token":"two","expires_in":3600}"#),
    ]);
    let exchange = fixture.exchange();
    let first = exchange
        .token(
            &input(ASSERTION_REF, Some(CLIENT), Some(TENANT), Some("scope-one")),
            &fixture.resolver(),
        )
        .unwrap();
    let second = exchange
        .token(
            &input(ASSERTION_REF, Some(CLIENT), Some(TENANT), Some("scope-two")),
            &fixture.resolver(),
        )
        .unwrap();
    assert_eq!(first.expose_secret(), "one");
    assert_eq!(second.expose_secret(), "two");
    assert_eq!(fixture.requests().len(), 2);
}

#[test]
fn azure_oidc_exchange_secret_safe_failure() {
    let secret = "fixture-assertion-secret";
    let fixture = Fixture::new(vec![Reply::status(400, r#"{"error":"invalid_client"}"#)])
        .with_environment(&[(ASSERTION_ENV, secret)]);
    let exchange = fixture.exchange();
    let debug = format!("{exchange:?}");
    assert!(debug.starts_with("AzureOidcExchange"));
    assert!(!debug.contains(secret));
    let error = exchange
        .token(&default_input(), &fixture.resolver())
        .unwrap_err();
    let shown = format!("{error} {error:?}");
    assert!(!shown.contains(secret));
    let requests = fixture.requests();
    let form_debug = format!("{:?}", requests[0].form);
    assert!(!form_debug.contains(secret));
    assert!(form_debug.contains("[REDACTED]"));

    let resolved = Fixture::new(vec![Reply::ok(TOKEN_BODY)]);
    let token = resolved
        .exchange()
        .token(&default_input(), &resolved.resolver())
        .unwrap();
    assert!(!format!("{token:?}").contains("tok-value"));

    let missing = Fixture::new(Vec::new());
    let unresolved = missing.error_of(&input(
        "os.environ/MISSING",
        Some(CLIENT),
        Some(TENANT),
        None,
    ));
    assert!(!format!("{unresolved} {unresolved:?}").contains(secret));
}

#[test]
fn replays_the_azure_oidc_exchange_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/azure_oidc_exchange");
    let mut replayed = 0_usize;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap();
            let decoded = decode_azure_oidc_token(&bytes);
            if name.starts_with("ok_") {
                assert!(decoded.is_ok(), "{name}");
            } else if name.starts_with("bad_") {
                assert!(decoded.is_err(), "{name}");
            } else {
                panic!("unexpected corpus category {name}");
            }
            replayed += 1_usize;
            Ok::<(), io::Error>(())
        })
        .unwrap();
    assert!(replayed > 0_usize);
}
