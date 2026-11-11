use std::sync::{Arc, Mutex};

use secrecy::{ExposeSecret as _, SecretString};

use crate::credential_lifecycle::outcome::SecretSave;
use crate::credential_lifecycle::save::save_cli_token;
use crate::credentials::contracts::CliTokenData;
use crate::credentials::storage::{StoredApiKeyRequest, get_stored_api_key};

use super::support::{
    FakeVault, FixedWallClock, Reply, SERVER, TestFiles, record, record_expiring_at, response,
    scripted_http, test_files, token_body,
};

async fn stored(files: &TestFiles, vault: &mut FakeVault, token: CliTokenData) {
    let outcome = save_cli_token(files, vault, token).await;
    assert!(matches!(outcome, SecretSave::SecretStored));
}

fn warn_sink(warnings: &Mutex<Vec<String>>) -> impl Fn(&str) + Send + Sync + '_ {
    move |message: &str| {
        warnings
            .lock()
            .expect("the warning log")
            .push(message.to_owned());
    }
}

fn frozen_clock() -> Arc<FixedWallClock> {
    Arc::new(FixedWallClock(1_000_000))
}

#[tokio::test]
async fn get_stored_api_key_returns_none_without_a_stored_login() {
    let files = test_files();
    let mut vault = FakeVault::working();
    let http = Arc::new(scripted_http(Vec::new()));
    let warnings = Mutex::new(Vec::new());

    let key = get_stored_api_key(StoredApiKeyRequest {
        expected_base_url: Some(SERVER),
        files: &files,
        vault: &mut vault,
        http: Arc::clone(&http),
        clock: frozen_clock(),
        warn: &warn_sink(&warnings),
        token_path: files.path(),
    })
    .await;

    assert!(key.is_none());
    assert!(http.requests().is_empty());
}

#[tokio::test]
async fn get_stored_api_key_returns_the_key_for_the_expected_origin() {
    let files = test_files();
    let mut vault = FakeVault::working();
    stored(&files, &mut vault, record()).await;
    let http = Arc::new(scripted_http(Vec::new()));
    let warnings = Mutex::new(Vec::new());

    let key = get_stored_api_key(StoredApiKeyRequest {
        expected_base_url: Some(SERVER),
        files: &files,
        vault: &mut vault,
        http: Arc::clone(&http),
        clock: frozen_clock(),
        warn: &warn_sink(&warnings),
        token_path: files.path(),
    })
    .await;

    assert_eq!(key.expect("the stored key").expose_secret(), "sk-secret");
    assert!(http.requests().is_empty());
}

#[tokio::test]
async fn get_stored_api_key_strips_trailing_slashes_from_the_expected_origin() {
    let files = test_files();
    let mut vault = FakeVault::working();
    stored(&files, &mut vault, record()).await;
    let http = Arc::new(scripted_http(Vec::new()));
    let warnings = Mutex::new(Vec::new());

    let key = get_stored_api_key(StoredApiKeyRequest {
        expected_base_url: Some("https://gateway.example///"),
        files: &files,
        vault: &mut vault,
        http: Arc::clone(&http),
        clock: frozen_clock(),
        warn: &warn_sink(&warnings),
        token_path: files.path(),
    })
    .await;

    assert_eq!(key.expect("the stored key").expose_secret(), "sk-secret");
}

#[tokio::test]
async fn get_stored_api_key_refuses_a_credential_for_another_origin() {
    let files = test_files();
    let mut vault = FakeVault::working();
    stored(&files, &mut vault, record()).await;
    let http = Arc::new(scripted_http(Vec::new()));
    let warnings = Mutex::new(Vec::new());

    let key = get_stored_api_key(StoredApiKeyRequest {
        expected_base_url: Some("https://other.example"),
        files: &files,
        vault: &mut vault,
        http: Arc::clone(&http),
        clock: frozen_clock(),
        warn: &warn_sink(&warnings),
        token_path: files.path(),
    })
    .await;

    assert!(key.is_none());
    assert!(http.requests().is_empty());
}

#[tokio::test]
async fn get_stored_api_key_refreshes_a_key_near_expiry() {
    let files = test_files();
    let mut vault = FakeVault::working();
    stored(&files, &mut vault, record_expiring_at(1_000_100.0_f64)).await;
    let http = Arc::new(scripted_http(vec![Reply::Response(response(
        200,
        &token_body("sk-refreshed", "refresh-2", 3_600),
    ))]));
    let warnings = Mutex::new(Vec::new());

    let key = get_stored_api_key(StoredApiKeyRequest {
        expected_base_url: None,
        files: &files,
        vault: &mut vault,
        http: Arc::clone(&http),
        clock: frozen_clock(),
        warn: &warn_sink(&warnings),
        token_path: files.path(),
    })
    .await;

    assert_eq!(
        key.expect("the renewed key").expose_secret(),
        "sk-refreshed"
    );
    assert_eq!(http.requests().len(), 1);
}

#[tokio::test]
async fn get_stored_api_key_returns_none_when_the_stored_key_is_empty() {
    let files = test_files();
    let mut vault = FakeVault::working();
    let mut empty = record();
    empty.key = Some(SecretString::from(String::new()));
    stored(&files, &mut vault, empty).await;
    let http = Arc::new(scripted_http(Vec::new()));
    let warnings = Mutex::new(Vec::new());

    let key = get_stored_api_key(StoredApiKeyRequest {
        expected_base_url: None,
        files: &files,
        vault: &mut vault,
        http: Arc::clone(&http),
        clock: frozen_clock(),
        warn: &warn_sink(&warnings),
        token_path: files.path(),
    })
    .await;

    assert!(key.is_none());
}

#[tokio::test]
async fn get_stored_api_key_resolves_the_secret_the_vault_keeps() {
    let files = test_files();
    let mut vault = FakeVault::working();
    stored(&files, &mut vault, record()).await;
    let http = Arc::new(scripted_http(Vec::new()));
    let warnings = Mutex::new(Vec::new());

    let key = get_stored_api_key(StoredApiKeyRequest {
        expected_base_url: Some(SERVER),
        files: &files,
        vault: &mut vault,
        http: Arc::clone(&http),
        clock: frozen_clock(),
        warn: &warn_sink(&warnings),
        token_path: files.path(),
    })
    .await;

    assert_eq!(key.expect("the stored key").expose_secret(), "sk-secret");
    let stored_metadata = files.stored().expect("the metadata file stays");
    assert!(stored_metadata.key.is_none());
    assert!(stored_metadata.refresh_token.is_none());
}
