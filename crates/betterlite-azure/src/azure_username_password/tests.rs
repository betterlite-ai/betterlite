#![cfg(test)]

use std::fs;
use std::io;
use std::path::Path;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_client_secret::transport::HttpReply;
use crate::azure_username_password::provider::UsernamePasswordProvider;
use crate::azure_username_password::token::decode_password_token;
use crate::azure_username_password::transport::HttpRequest;

use fixture::{
    CLIENT, Fixture, HOST, Reply, USERNAME, form_map, input, input_for, openid, scope_set, token,
};
use scenarios::token_body;

mod fixture;
mod logs;
mod realm;
mod scenarios;

#[rstest]
#[case::managed_accounts_proceed(realm::managed_accounts_proceed)]
#[case::federated_account_is_refused(realm::federated_account_is_refused)]
#[case::realm_failures_stop_the_grant(realm::realm_failures_stop_the_grant)]
fn azure_username_password_realm_discovery(#[case] scenario: fn()) {
    scenario();
}

#[rstest]
#[case::password_grant_form(scenarios::password_grant_form)]
#[case::default_tenant(scenarios::default_tenant)]
#[case::invalid_grant(scenarios::invalid_grant)]
#[case::token_response(scenarios::token_response)]
#[case::username_debug_warning(scenarios::username_debug_warning)]
#[case::username_constructor_failure(scenarios::username_constructor_failure)]
#[case::password_scope_decoration(scenarios::password_scope_decoration)]
fn azure_username_password_cases(#[case] scenario: fn()) {
    scenario();
}

fn identity_body(token: &str) -> String {
    format!(
        r#"{{"access_token":"{token}","expires_in":3600,"id_token_claims":{{"aud":"client","iss":"https://login.microsoftonline.com/tenant/v2.0","sub":"subject","tid":"tenant","preferred_username":"user@example.com"}},"client_info":"eyJ1aWQiOiJ1aWQiLCJ1dGlkIjoidXRpZCJ9"}}"#
    )
}

fn assert_identity(
    request: &HttpRequest,
    client: &str,
    username: &str,
    password: &str,
    scope: &str,
) {
    let form = form_map(request);
    assert_eq!(form.get("client_id").map(String::as_str), Some(client));
    assert_eq!(form.get("username").map(String::as_str), Some(username));
    assert_eq!(form.get("password").map(String::as_str), Some(password));
    assert!(
        scope_set(request).iter().any(|entry| entry == scope),
        "{scope}"
    );
}

#[test]
fn azure_username_password_credential_identity_isolation() {
    let fixture = Fixture::new(vec![
        openid(HOST),
        token(&identity_body("token-one")),
        openid(HOST),
        token(&identity_body("token-two")),
    ]);
    let first = identity(&fixture, "one");
    let second = identity(&fixture, "two");
    assert_eq!(first.token().unwrap().token.expose_secret(), "token-one");
    assert_eq!(second.token().unwrap().token.expose_secret(), "token-two");
    let posts = fixture.transport.posts();
    assert_eq!(posts.len(), 2);
    assert_identity(
        &posts[0],
        "client-one",
        "user-one",
        "password-one",
        "scope-one",
    );
    assert_identity(
        &posts[1],
        "client-two",
        "user-two",
        "password-two",
        "scope-two",
    );
    assert_ne!(posts[0].form, posts[1].form);
}

fn identity(fixture: &Fixture, word: &str) -> UsernamePasswordProvider {
    fixture
        .provider(&input(
            &format!("client-{word}"),
            &format!("user-{word}"),
            &format!("password-{word}"),
            Some(&format!("scope-{word}")),
        ))
        .unwrap()
}

#[test]
fn azure_username_password_secret_safe_failure() {
    let secret = "fixture-secret-value";
    failure_path_is_safe(secret);
    token_and_reply_debug_redact(secret);
}

fn failure_path_is_safe(secret: &str) {
    let fixture = Fixture::new(vec![
        openid(HOST),
        Reply::status(
            400,
            "/oauth2/v2.0/token",
            r#"{"error":"invalid_grant","error_description":"wrong password"}"#,
        ),
    ]);
    let provider = fixture
        .provider(&input_for(USERNAME, secret, None))
        .unwrap();
    let provider_debug = format!("{provider:?}");
    assert!(provider_debug.starts_with("UsernamePasswordProvider"));
    assert!(provider_debug.contains(CLIENT));
    assert!(provider_debug.contains(HOST));
    assert!(!provider_debug.contains(secret));
    assert!(!provider_debug.contains(USERNAME));
    let error = provider.token().unwrap_err();
    let shown = format!("{error} {error:?}");
    assert!(!shown.contains(secret));
    let request = fixture.transport.posts().remove(0);
    let debug = format!("{request:?}");
    assert!(!debug.contains(secret));
    assert!(debug.contains("[redacted]"));
}

fn token_and_reply_debug_redact(secret: &str) {
    let successful = Fixture::new(vec![openid(HOST), token(&token_body())]);
    let access = successful
        .provider(&input_for(USERNAME, secret, None))
        .unwrap()
        .token()
        .unwrap();
    assert_eq!(access.token.expose_secret(), "tok-1");
    let token_debug = format!("{access:?}");
    assert!(!token_debug.contains("tok-1"));
    assert!(token_debug.contains("[REDACTED]"));
    let reply = HttpReply {
        status: 200,
        body: SecretString::from("fixture-token"),
    };
    let reply_debug = format!("{reply:?}");
    assert!(!reply_debug.contains("fixture-token"));
    assert!(reply_debug.contains("[REDACTED]"));
}

#[test]
fn replays_the_azure_username_password_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/azure_username_password");
    let mut replayed = 0_usize;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap();
            let decoded = decode_password_token(&bytes);
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
