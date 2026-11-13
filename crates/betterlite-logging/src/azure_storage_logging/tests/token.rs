#![cfg(test)]

use std::collections::BTreeMap;

use url::form_urlencoded;

use super::support::{ACCOUNT, ENTRA, ManualClock, Reply, ScriptedTransport, fixture};
use crate::azure_storage_logging::token::{REFRESH_MARGIN, STORAGE_SCOPE, TOKEN_LIFETIME};

const TOKEN_ROUTE: &str = "https://login.microsoftonline.com/tenant-1/oauth2/v2.0/token";

fn token_reply() -> Reply {
    Reply {
        status: 200,
        body: r#"{"access_token": "tok-1"}"#,
    }
}

fn form(call: &super::support::Call) -> BTreeMap<String, String> {
    form_urlencoded::parse(&call.body).into_owned().collect()
}

#[tokio::test]
async fn the_client_secret_path_posts_the_entra_form_once_and_caches_the_token() {
    let fixture = fixture(&ENTRA, &[token_reply(), token_reply()]);
    let first = fixture
        .logger
        .set_valid_azure_ad_token()
        .await
        .expect("a scripted token is returned");
    assert_eq!(secrecy::ExposeSecret::expose_secret(&first), "tok-1");
    let calls = fixture.transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "POST");
    assert_eq!(calls[0].url, TOKEN_ROUTE);
    let fields = form(&calls[0]);
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
    assert_eq!(secrecy::ExposeSecret::expose_secret(&second), "tok-1");
    assert_eq!(fixture.transport.calls().len(), 1);
}

#[tokio::test]
async fn a_token_refreshes_exactly_five_minutes_before_it_expires() {
    let fixture = fixture(&ENTRA, &[token_reply(), token_reply()]);
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
    fixture
        .logger
        .set_valid_azure_ad_token()
        .await
        .expect("chain token");
    fixture
        .logger
        .set_valid_azure_ad_token()
        .await
        .expect("chain token");
    assert_eq!(*fixture.factory.built.lock().expect("factory count"), 1);
    assert_eq!(*fixture.chain.calls.lock().expect("provider count"), 2);
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
    let runtime = tokio::runtime::Builder::new_current_thread()
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

#[test]
fn the_payload_fixture_round_trips_through_json() {
    let payload = super::support::payload("id-1");
    let encoded = serde_json::to_value(&payload).expect("serializes");
    assert_eq!(
        encoded.get("status").and_then(serde_json::Value::as_str),
        Some("success")
    );
}
