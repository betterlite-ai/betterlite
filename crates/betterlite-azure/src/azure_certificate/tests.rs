#![cfg(test)]

use std::env::temp_dir;
use std::error::Error as _;
use std::fs;
use std::io;
use std::path::Path;
use std::process::id;
use std::sync::atomic::Ordering;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_certificate::CertificateError;
use crate::azure_certificate::assertion::{IdSource as _, RandomIds};
use crate::azure_certificate::credential::{FileReader as _, StdFiles};
use crate::azure_certificate::material::decode_certificate_material;
use crate::azure_client_secret::token::AccessToken;
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

const ERROR_REPLY: &str = r#"{"error":"temporarily_unavailable"}"#;

fn aging(reply: Option<(u16, &str)>, at: i64) -> Result<AccessToken, CertificateError> {
    let first = r#"{"access_token":"a","expires_in":3600,"refresh_in":30}"#;
    let mut replies = vec![openid(TENANT), token_reply(first)];
    replies.extend(reply.map(|(status, body)| (status, body.to_owned())));
    let fixture = Fixture::new(replies);
    let credential = fixture.credential(input(RSA_PKCS8))?;
    credential.token()?;
    fixture.clock.0.store(at, Ordering::SeqCst);
    credential.token()
}

#[rstest]
#[case::idp_error(Some((400, ERROR_REPLY)), NOW + 30)]
#[case::empty_reply(Some((400, "{}")), NOW + 30)]
#[case::last_valid_second(Some((400, ERROR_REPLY)), NOW + 3599)]
fn azure_certificate_serves_aging_token(#[case] reply: Option<(u16, &str)>, #[case] at: i64) {
    assert_eq!(aging(reply, at).unwrap().token.expose_secret(), "a");
}

#[rstest]
#[case::expired(Some((400, ERROR_REPLY)), NOW + 3600, "Authentication failed: temporarily_unavailable")]
#[case::description_only(Some((400, r#"{"error_description":"throttled"}"#)), NOW + 30, "Authentication failed: throttled")]
#[case::server_error(Some((500, "busy")), NOW + 30, "Authentication failed: HTTP Error: 500")]
#[case::not_json(Some((400, "busy")), NOW + 30, "the token response is not JSON")]
#[case::transport_failure(None, NOW + 30, "Authentication failed: no reply left")]
fn azure_certificate_refuses_without_serving_the_aging_token(
    #[case] reply: Option<(u16, &str)>,
    #[case] at: i64,
    #[case] message: &str,
) {
    assert_eq!(aging(reply, at).unwrap_err().to_string(), message);
}

#[rstest]
#[case::pkcs12(b"\x00\x01\x02not a keystore".as_slice(), None)]
#[case::pem_framing(b"-----BEGIN PRIVATE KEY-----\nAAAA\n-----END CERTIFICATE-----\n".as_slice(), None)]
#[case::key_data(b"-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n".as_slice(), None)]
#[case::bad_password(RSA_ENCRYPTED, Some("wrong"))]
fn azure_certificate_keeps_the_library_cause(#[case] data: &[u8], #[case] password: Option<&str>) {
    let secret = password.map(SecretString::from);
    let error = decode_certificate_material(data, secret.as_ref()).unwrap_err();
    assert!(error.source().is_some(), "{error}");
}

#[test]
fn azure_certificate_default_seams() {
    let first = RandomIds.next_id();
    let second = RandomIds.next_id();
    assert_ne!(first, second);
    assert_eq!(uuid::Uuid::parse_str(&first).unwrap().get_version_num(), 4);

    let folder = temp_dir().join(format!("betterlite-azure-{}", id()));
    fs::create_dir_all(&folder).unwrap();
    let file = folder.join("leaf.pem");
    fs::write(&file, RSA_PKCS8).unwrap();
    let read = StdFiles.read(file.to_str().unwrap()).unwrap();
    assert_eq!(read, RSA_PKCS8);
    let missing = StdFiles.read(folder.join("absent.pem").to_str().unwrap());
    assert_eq!(missing.unwrap_err().kind(), io::ErrorKind::NotFound);
    fs::remove_dir_all(&folder).unwrap();
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
