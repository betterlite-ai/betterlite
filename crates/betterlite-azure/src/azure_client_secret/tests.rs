#![cfg(test)]

use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_client_secret::authority;
use crate::azure_client_secret::provider::ClientSecretProvider;
use crate::azure_client_secret::token::decode_client_secret_token;
use crate::azure_client_secret::transport::{HttpReply, HttpRequest};
use fixture::{
    CLIENT, Fixture, HOST, Reply, TENANT, form_map, input_for, instance_metadata, openid, token,
};

mod fixture;
mod scenarios;

#[rstest]
#[case::client_secret_form(scenarios::client_secret_form)]
#[case::tenant_scope(scenarios::tenant_scope)]
#[case::oauth_failure(scenarios::oauth_failure)]
#[case::token_expiry(scenarios::token_expiry)]
#[case::client_secret_reference_resolution(scenarios::client_secret_reference_resolution)]
#[case::client_secret_provider_cache(scenarios::client_secret_provider_cache)]
#[case::azure_error_fields(scenarios::azure_error_fields)]
#[case::regional_authority_inputs(scenarios::regional_authority_inputs)]
#[case::regional_authority_discovery(scenarios::regional_authority_discovery)]
#[case::regional_authority_host(scenarios::regional_authority_host)]
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
    let provider_debug = format!("{provider:?}");
    assert!(provider_debug.starts_with("ClientSecretProvider"));
    assert!(provider_debug.contains(TENANT));
    assert!(provider_debug.contains(CLIENT));
    assert!(provider_debug.contains(HOST));
    assert!(!provider_debug.contains(secret));
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

const INSTANCE_DISCOVERY: &str = "https://login.microsoftonline.com/common/discovery/instance";

#[rstest]
#[case::b2c_policy_segment(
    "https://login.contoso.test/tenant",
    "B2C_1_signin",
    1,
    "https://login.contoso.test/tenant/B2C_1_signin/v2.0/.well-known/openid-configuration"
)]
#[case::three_segments_without_policy(
    "https://login.contoso.test/tenant",
    "other",
    2,
    INSTANCE_DISCOVERY
)]
#[case::policy_before_last_segment(
    "https://login.contoso.test/tenant/b2c_policy",
    "extra",
    2,
    INSTANCE_DISCOVERY
)]
#[case::adfs_without_version(
    "https://adfs.contoso.test",
    "ADFS",
    1,
    "https://adfs.contoso.test/ADFS/.well-known/openid-configuration"
)]
#[case::b2c_host(
    "https://contoso.b2clogin.com",
    TENANT,
    1,
    "https://contoso.b2clogin.com/tenant/v2.0/.well-known/openid-configuration"
)]
#[case::ciam_without_tenant(
    "https://contoso.ciamlogin.com",
    "",
    1,
    "https://contoso.ciamlogin.com/contoso.onmicrosoft.com/v2.0/.well-known/openid-configuration"
)]
#[case::ciam_with_path(
    "https://contoso.ciamlogin.com/extra",
    "tenant",
    1,
    "https://contoso.ciamlogin.com/extra/tenant/v2.0/.well-known/openid-configuration"
)]
fn azure_client_secret_authority_flavours(
    #[case] authority_host: &str,
    #[case] tenant_id: &str,
    #[case] request_count: usize,
    #[case] first_url: &str,
) {
    let fixture = Fixture::new(vec![
        instance_metadata(
            r#"{"tenant_discovery_endpoint":"https://login.contoso.test/tenant/v2.0/.well-known/openid-configuration"}"#,
        ),
        openid("login.contoso.test", TENANT),
    ]);
    authority::discover_authority(&*fixture.transport, authority_host, tenant_id, true).unwrap();
    let requests = fixture.transport.requests();
    assert_eq!(requests.len(), request_count);
    assert_eq!(requests[0].url, first_url);
}
