#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use rstest::rstest;
use secrecy::ExposeSecret as _;
use tokio::runtime::Builder;
use url::form_urlencoded;

use super::support::{ACCOUNT, ENTRA, Reply, environment, fixture};
use crate::azure_storage_logging::chain::UnwiredChainFactory;
use crate::azure_storage_logging::token::{
    AUTHORITY_HOST_VARIABLE, ChainProviderFactory as _, DEFAULT_AUTHORITY_HOST, REFRESH_MARGIN,
    STORAGE_SCOPE, TOKEN_LIFETIME, authority_host, normalize_authority, parse_token_response,
};
use crate::azure_storage_logging::transport::StorageResponse;

const TOKEN_ROUTE: &str = "https://login.microsoftonline.com/tenant-1/oauth2/v2.0/token";

#[rstest]
#[case::nothing_configured(&[], "no available credential")]
#[case::an_empty_value_is_unset(&[("AZURE_CLIENT_ID", "")], "no available credential")]
#[case::a_workload_identity_file(&[("AZURE_FEDERATED_TOKEN_FILE", "/token")], "not wired")]
fn the_unwired_chain_says_why_it_has_no_token(
    #[case] variables: &[(&str, &str)],
    #[case] expected: &str,
) {
    let factory = UnwiredChainFactory {
        environment: Arc::new(environment(variables)),
    };
    let error = factory.provider().err().expect("the chain is not wired");
    assert!(error.to_string().contains(expected));
}

#[tokio::test]
async fn the_client_secret_path_posts_the_entra_form_once_and_caches_the_token() {
    let fixture = fixture(&ENTRA, &[Reply::TOKEN, Reply::TOKEN]);
    let first = fixture
        .logger
        .set_valid_azure_ad_token()
        .await
        .expect("a scripted token is returned");
    assert_eq!(first.expose_secret(), "tok-1");
    let calls = fixture.transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "POST");
    assert_eq!(calls[0].url, TOKEN_ROUTE);
    let fields: BTreeMap<String, String> = form_urlencoded::parse(&calls[0].body)
        .into_owned()
        .collect();
    assert_eq!(
        fields.get("grant_type").map(String::as_str),
        Some("client_credentials")
    );
    assert_eq!(
        fields.get("client_id").map(String::as_str),
        Some("client-1")
    );
    assert_eq!(
        fields.get("client_secret").map(String::as_str),
        Some("secret-1")
    );
    assert_eq!(fields.get("scope").map(String::as_str), Some(STORAGE_SCOPE));
    let second = fixture
        .logger
        .set_valid_azure_ad_token()
        .await
        .expect("the cached token is returned");
    assert_eq!(second.expose_secret(), "tok-1");
    assert_eq!(fixture.transport.calls().len(), 1);
}

#[tokio::test]
async fn a_token_refreshes_exactly_five_minutes_before_it_expires() {
    let fixture = fixture(&ENTRA, &[Reply::TOKEN, Reply::TOKEN]);
    fixture
        .logger
        .set_valid_azure_ad_token()
        .await
        .expect("token");
    let lifetime = TOKEN_LIFETIME.as_secs();
    fixture.clock.advance(
        lifetime
            .saturating_sub(REFRESH_MARGIN.as_secs())
            .saturating_sub(1),
    );
    fixture
        .logger
        .set_valid_azure_ad_token()
        .await
        .expect("token");
    assert_eq!(fixture.transport.calls().len(), 1);
    fixture.clock.advance(1);
    fixture
        .logger
        .set_valid_azure_ad_token()
        .await
        .expect("token");
    assert_eq!(fixture.transport.calls().len(), 2);
}

#[tokio::test]
async fn a_failed_token_response_is_an_error() {
    let fixture = fixture(
        &ENTRA,
        &[Reply {
            status: 401,
            body: "denied",
        }],
    );
    let error = fixture
        .logger
        .set_valid_azure_ad_token()
        .await
        .expect_err("a 401 is not a token");
    assert!(error.to_string().contains("401"));
}

#[tokio::test]
async fn the_credential_chain_factory_is_cached_and_its_provider_runs_per_upload() {
    let fixture = fixture(&ACCOUNT[..2], &[]);
    let first = fixture
        .logger
        .set_valid_azure_ad_token()
        .await
        .expect("chain token");
    assert_eq!(first.expose_secret(), "chain-token");
    fixture
        .logger
        .set_valid_azure_ad_token()
        .await
        .expect("chain token");
    assert_eq!(fixture.factory.built.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.chain.calls.load(Ordering::SeqCst), 2);
    assert!(fixture.transport.calls().is_empty());
}

#[test]
fn a_missing_service_principal_value_keeps_litellms_error() {
    let variables = [
        ("AZURE_STORAGE_ACCOUNT_NAME", "acct"),
        ("AZURE_STORAGE_FILE_SYSTEM", "logs"),
        ("AZURE_STORAGE_TENANT_ID", "tenant-1"),
    ];
    let fixture = fixture(&variables, &[]);
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let error = runtime
        .block_on(fixture.logger.set_valid_azure_ad_token())
        .expect_err("the client id is required");
    assert_eq!(
        error.to_string(),
        "Missing required environment variable: AZURE_STORAGE_CLIENT_ID"
    );
    assert!(fixture.transport.calls().is_empty());
}

#[rstest]
#[case::a_sovereign_cloud("https://login.microsoftonline.us", "https://login.microsoftonline.us")]
#[case::a_trailing_slash(
    "https://login.microsoftonline.us/",
    "https://login.microsoftonline.us"
)]
#[case::a_bare_host("login.microsoftonline.us", "https://login.microsoftonline.us")]
fn the_authority_is_normalized_like_azure_identity(#[case] host: &str, #[case] expected: &str) {
    assert_eq!(
        normalize_authority(host).expect("a valid authority"),
        expected
    );
}

#[test]
fn an_http_authority_is_rejected() {
    let error = normalize_authority("http://login.microsoftonline.us").expect_err("https only");
    assert_eq!(
        error.to_string(),
        "'http://login.microsoftonline.us' is an invalid authority. The value must be a TLS protected (https) URL."
    );
}

#[test]
fn an_empty_authority_host_counts_as_unset() {
    assert_eq!(
        authority_host(&environment(&[(AUTHORITY_HOST_VARIABLE, "")])),
        DEFAULT_AUTHORITY_HOST
    );
}

#[tokio::test]
async fn the_authority_host_comes_from_azure_authority_host() {
    let mut variables = ENTRA.to_vec();
    variables.push(("AZURE_AUTHORITY_HOST", "https://login.microsoftonline.us/"));
    let fixture = fixture(&variables, &[Reply::TOKEN]);
    let token = fixture
        .logger
        .set_valid_azure_ad_token()
        .await
        .expect("a scripted token is returned");
    assert_eq!(token.expose_secret(), "tok-1");
    assert_eq!(
        fixture.transport.calls()[0].url,
        "https://login.microsoftonline.us/tenant-1/oauth2/v2.0/token"
    );
}

#[tokio::test]
async fn os_environ_references_are_resolved_before_the_token_request() {
    let variables = [
        ("AZURE_STORAGE_ACCOUNT_NAME", "acct"),
        ("AZURE_STORAGE_FILE_SYSTEM", "logs"),
        ("AZURE_STORAGE_TENANT_ID", "os.environ/STORAGE_TENANT"),
        ("AZURE_STORAGE_CLIENT_ID", "os.environ/STORAGE_CLIENT"),
        ("AZURE_STORAGE_CLIENT_SECRET", "os.environ/STORAGE_SECRET"),
        ("STORAGE_TENANT", "tenant-1"),
        ("STORAGE_CLIENT", "client-1"),
        ("STORAGE_SECRET", "secret-1"),
    ];
    let fixture = fixture(&variables, &[Reply::TOKEN]);
    fixture
        .logger
        .set_valid_azure_ad_token()
        .await
        .expect("a scripted token is returned");
    let calls = fixture.transport.calls();
    assert_eq!(calls[0].url, TOKEN_ROUTE);
    let fields: BTreeMap<String, String> = form_urlencoded::parse(&calls[0].body)
        .into_owned()
        .collect();
    assert_eq!(
        fields.get("client_id").map(String::as_str),
        Some("client-1")
    );
    assert_eq!(
        fields.get("client_secret").map(String::as_str),
        Some("secret-1")
    );
}

#[tokio::test]
async fn an_unresolved_os_environ_reference_keeps_litellms_error() {
    let variables = [
        ("AZURE_STORAGE_ACCOUNT_NAME", "acct"),
        ("AZURE_STORAGE_FILE_SYSTEM", "logs"),
        ("AZURE_STORAGE_TENANT_ID", "os.environ/STORAGE_TENANT"),
        ("AZURE_STORAGE_CLIENT_ID", "client-1"),
        ("AZURE_STORAGE_CLIENT_SECRET", "os.environ/STORAGE_SECRET"),
    ];
    let fixture = fixture(&variables, &[]);
    let error = fixture
        .logger
        .set_valid_azure_ad_token()
        .await
        .expect_err("the tenant reference resolves to nothing");
    assert_eq!(
        error.to_string(),
        "tenant_id, client_id, and client_secret must be provided"
    );
    assert!(fixture.transport.calls().is_empty());
}

#[tokio::test]
async fn an_empty_os_environ_reference_counts_as_unset() {
    let variables = [
        ("AZURE_STORAGE_ACCOUNT_NAME", "acct"),
        ("AZURE_STORAGE_FILE_SYSTEM", "logs"),
        ("AZURE_STORAGE_TENANT_ID", "os.environ/STORAGE_TENANT"),
        ("AZURE_STORAGE_CLIENT_ID", "client-1"),
        ("AZURE_STORAGE_CLIENT_SECRET", "secret-1"),
        ("STORAGE_TENANT", ""),
    ];
    let fixture = fixture(&variables, &[]);
    let error = fixture
        .logger
        .set_valid_azure_ad_token()
        .await
        .expect_err("the tenant reference is empty");
    assert_eq!(
        error.to_string(),
        "tenant_id, client_id, and client_secret must be provided"
    );
    assert!(fixture.transport.calls().is_empty());
}

#[rstest]
fn replays_the_azure_storage_token_response_corpus(
    #[files("fuzz/corpus/azure_storage_token_response/*")] path: PathBuf,
) {
    let body = fs::read(&path).expect("the corpus file");
    let response = StorageResponse { status: 200, body };
    let _token = parse_token_response(&response);
}
