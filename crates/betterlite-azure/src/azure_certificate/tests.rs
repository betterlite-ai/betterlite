#![cfg(test)]

use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::Ordering;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_certificate::material::decode_certificate_material;
use crate::azure_client_secret::transport::HttpRequest;
use fixture::{
    CLIENT, Fixture, NOW, PASSWORD, RSA_ENCRYPTED, RSA_PKCS8, TENANT, decode, form, input, openid,
    token_reply, with_password,
};

mod fixture;
mod scenarios;

#[rstest]
#[case::pem_certificate(scenarios::pem_certificate)]
#[case::pkcs12_password(scenarios::pkcs12_password)]
#[case::client_assertion_claims(scenarios::client_assertion_claims)]
#[case::invalid_certificate(scenarios::invalid_certificate)]
#[case::token_response(scenarios::token_response)]
#[case::certificate_argument_boundary(scenarios::certificate_argument_boundary)]
#[case::certificate_chain_header(scenarios::certificate_chain_header)]
fn azure_certificate_cases(#[case] scenario: fn()) {
    scenario();
}

#[test]
fn azure_certificate_credential_identity_isolation() {
    let fixture = Fixture::new(vec![
        openid("tenant-one"),
        token_reply(r#"{"access_token":"token-one","expires_in":3600}"#),
        openid("tenant-two"),
        token_reply(r#"{"access_token":"token-two","expires_in":3600}"#),
    ]);
    let mut one = input(RSA_PKCS8);
    one.tenant_id = "tenant-one".to_owned();
    one.client_id = "client-one".to_owned();
    one.scope = Some("scope-one".to_owned());
    let mut two = input(RSA_PKCS8);
    two.tenant_id = "tenant-two".to_owned();
    two.client_id = "client-two".to_owned();
    two.scope = Some("scope-two".to_owned());
    let first = fixture.credential(one).unwrap();
    let second = fixture.credential(two).unwrap();
    assert_eq!(first.token().unwrap().token.expose_secret(), "token-one");
    assert_eq!(second.token().unwrap().token.expose_secret(), "token-two");
    let posts = fixture.transport.posts();
    assert_eq!(posts.len(), 2);
    assert_identity(&posts[0], "tenant-one", "client-one", "scope-one");
    assert_identity(&posts[1], "tenant-two", "client-two", "scope-two");
    // each credential keeps its own token
    assert_eq!(first.token().unwrap().token.expose_secret(), "token-one");
    assert_eq!(fixture.transport.posts().len(), 2);
}

fn assert_identity(post: &HttpRequest, tenant: &str, client: &str, scope: &str) {
    let fields = form(post);
    assert!(post.url.contains(tenant));
    assert_eq!(fields["client_id"], client);
    assert_eq!(fields["scope"], scope);
    let (_, claims) = decode(&fields["client_assertion"]);
    assert_eq!(claims.iss, client);
    assert_eq!(claims.sub, client);
    assert_eq!(claims.aud, post.url);
}

#[test]
fn azure_certificate_secret_safe_failure() {
    let wrong = "sentinel-wrong-password";
    let unused = Fixture::new(Vec::new());
    let rejected = unused
        .credential(with_password(input(RSA_ENCRYPTED), wrong))
        .unwrap_err();
    assert!(!format!("{rejected} {rejected:?}").contains(wrong));

    let replies = vec![
        openid(TENANT),
        token_reply(r#"{"access_token":"sentinel-token","expires_in":3600}"#),
        (400, r#"{"error_description":"denied"}"#.to_owned()),
    ];
    let fixture = Fixture::new(replies);
    let credential = fixture
        .credential(with_password(input(RSA_ENCRYPTED), PASSWORD))
        .unwrap();
    let identity = format!("{credential:?}");
    assert!(identity.starts_with("CertificateCredential"));
    assert!(identity.contains(TENANT));
    assert!(identity.contains(CLIENT));
    assert!(!identity.contains(PASSWORD));
    let issued = credential.token().unwrap();
    let issued_debug = format!("{issued:?} {credential:?}");
    assert!(!issued_debug.contains("sentinel-token"));
    assert!(issued_debug.contains("[REDACTED]"));
    let first_assertion = form(&fixture.transport.posts()[0])["client_assertion"].clone();
    assert!(!issued_debug.contains(&first_assertion));

    fixture.clock.0.store(NOW + 7200, Ordering::SeqCst);
    let denied = credential.token().unwrap_err();
    assert_eq!(denied.to_string(), "Authentication failed: denied");
    let denied_text = format!("{denied} {denied:?}");
    assert!(!denied_text.contains("sentinel-token"));
    assert!(!denied_text.contains(PASSWORD));
    let second_assertion = form(&fixture.transport.posts()[1])["client_assertion"].clone();
    assert!(!denied_text.contains(&second_assertion));
}

#[test]
fn replays_the_azure_certificate_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/azure_certificate");
    let mut replayed = 0_usize;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap();
            // seeds named locked_ are protected by the fixture password
            let password = name
                .contains("locked")
                .then(|| SecretString::from(PASSWORD));
            let decoded = decode_certificate_material(&bytes, password.as_ref());
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
