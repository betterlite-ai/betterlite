#![cfg(test)]

use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_client_secret::provider::ClientSecretProvider;
use crate::azure_client_secret::token::decode_client_secret_token;
use crate::azure_client_secret::transport::{HttpReply, HttpRequest};
use fixture::{CLIENT, Fixture, HOST, Reply, TENANT, form_map, input_for, openid, token};

mod fixture;
mod scenarios;

#[rstest]
#[case::case_1_client_secret_form(scenarios::client_secret_form)]
#[case::case_2_tenant_scope(scenarios::tenant_scope)]
#[case::case_3_oauth_failure(scenarios::oauth_failure)]
#[case::case_4_token_expiry(scenarios::token_expiry)]
#[case::case_5_client_secret_reference_resolution(scenarios::client_secret_reference_resolution)]
#[case::case_6_client_secret_provider_cache(scenarios::client_secret_provider_cache)]
#[case::case_7_azure_error_fields(scenarios::azure_error_fields)]
#[case::case_8_regional_authority_inputs(scenarios::regional_authority_inputs)]
#[case::case_9_regional_authority_discovery(scenarios::regional_authority_discovery)]
#[case::case_10_regional_authority_host(scenarios::regional_authority_host)]
fn azure_client_secret_cases(#[case] scenario: fn()) {
    scenario();
}

#[test]
fn azure_client_secret_credential_identity_isolation() {
    isolation_cache();
    isolation_requests();
}

fn isolation_cache() {
    let fixture = Fixture::new(Vec::new());
    let mut cache = fixture.cache();
    let first = cache
        .provider(
            &input_for("tenant-one", "client-one", "secret-one", Some("scope-one")),
            &fixture.resolver,
            None,
        )
        .unwrap();
    let second = cache
        .provider(
            &input_for("tenant-two", "client-two", "secret-two", Some("scope-two")),
            &fixture.resolver,
            None,
        )
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &second));
    let again = cache
        .provider(
            &input_for("tenant-two", "client-two", "secret-two", Some("scope-two")),
            &fixture.resolver,
            None,
        )
        .unwrap();
    assert!(Arc::ptr_eq(&again, &second));
}

fn two_providers(fixture: &Fixture) -> (Arc<ClientSecretProvider>, Arc<ClientSecretProvider>) {
    let mut cache = fixture.cache();
    let first = cache
        .provider(
            &input_for("tenant-one", "client-one", "secret-one", Some("scope-one")),
            &fixture.resolver,
            None,
        )
        .unwrap();
    let second = cache
        .provider(
            &input_for("tenant-two", "client-two", "secret-two", Some("scope-two")),
            &fixture.resolver,
            None,
        )
        .unwrap();
    (first, second)
}

fn isolation_requests() {
    let fixture = Fixture::new(vec![
        openid(HOST, "tenant-one"),
        token(r#"{"access_token":"token-one","expires_in":3600}"#),
        openid(HOST, "tenant-two"),
        token(r#"{"access_token":"token-two","expires_in":3600}"#),
    ]);
    let (first, second) = two_providers(&fixture);
    let first_token = first.token().unwrap();
    let second_token = second.token().unwrap();
    assert_eq!(first_token.token.expose_secret(), "token-one");
    assert_eq!(second_token.token.expose_secret(), "token-two");
    let posts = fixture.transport.posts();
    assert_eq!(posts.len(), 2);
    assert_identity(
        &posts[0],
        "tenant-one",
        "client-one",
        "secret-one",
        "scope-one",
    );
    assert_identity(
        &posts[1],
        "tenant-two",
        "client-two",
        "secret-two",
        "scope-two",
    );
}

fn assert_identity(request: &HttpRequest, tenant: &str, client: &str, secret: &str, scope: &str) {
    let form = form_map(request);
    assert!(request.url.contains(tenant));
    assert_eq!(form.get("client_id").map(String::as_str), Some(client));
    assert_eq!(form.get("client_secret").map(String::as_str), Some(secret));
    assert_eq!(form.get("scope").map(String::as_str), Some(scope));
}

#[test]
fn azure_client_secret_secret_safe_failure() {
    let secret = "fixture-secret-value";
    let fixture = Fixture::new(vec![
        openid(HOST, TENANT),
        Reply::status(400, "/oauth2/v2.0/token", r#"{"error":"invalid_client"}"#),
    ]);
    let provider = fixture.provider_for(&input_for(TENANT, CLIENT, secret, None), None);
    let error = provider.token().unwrap_err();
    let shown = format!("{error} {error:?}");
    assert!(!shown.contains(secret));
    let request = fixture.transport.posts().remove(0);
    let debug = format!("{request:?}");
    assert!(!debug.contains(secret));
    assert!(debug.contains("[redacted]"));

    let successful = Fixture::new(vec![
        openid(HOST, TENANT),
        token(r#"{"access_token":"fixture-token","expires_in":3600}"#),
    ]);
    let access = successful.provider().token().unwrap();
    assert_eq!(access.token.expose_secret(), "fixture-token");
    let reply = HttpReply {
        status: 200,
        body: SecretString::from("fixture-token"),
    };
    let reply_debug = format!("{reply:?}");
    assert!(!reply_debug.contains("fixture-token"));
    assert!(reply_debug.contains("[REDACTED]"));
    let token_debug = format!("{access:?}");
    assert!(!token_debug.contains("fixture-token"));
    assert!(token_debug.contains("[REDACTED]"));
}

#[test]
fn replays_the_azure_client_secret_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/azure_client_secret");
    let mut replayed = 0_usize;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap();
            let decoded = decode_client_secret_token(&bytes);
            match name.split('_').nth(1).unwrap() {
                "ok" => assert!(decoded.is_ok(), "{name}"),
                "bad" => assert!(decoded.is_err(), "{name}"),
                other => panic!("unexpected corpus category {other}"),
            }
            replayed += 1_usize;
            Ok::<(), io::Error>(())
        })
        .unwrap();
    assert!(replayed > 0_usize);
}
