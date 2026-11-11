use std::sync::Mutex;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::credential_lifecycle::outcome::SecretSave;
use crate::credential_lifecycle::save::save_cli_token;
use crate::credentials::contracts::CliTokenData;
use crate::credentials::renewal::{
    FreshApiKeyRequest, RenewalStore as _, RenewalVault, fresh_api_key, key_rotated_by_a_sibling,
    refresh_inputs,
};
use crate::credentials::storage::CredentialStore;
use crate::keyring::KeyringUnusable;

use super::support::{
    FakeVault, FixedWallClock, OTHER_SERVER, RESOURCE, REVOCATION_ENDPOINT, Refusal, Reply, SERVER,
    ScriptedHttp, ScriptedStore, TOKEN_ENDPOINT, form_fields, form_value, record,
    record_expiring_at, response, scripted_http, scripted_store, secret, test_files, token_body,
};

const NOW: i64 = 1_000_000;
const SIBLING_NOW: f64 = 1_000.0_f64;

fn warn_sink(log: &Mutex<Vec<String>>) -> impl Fn(&str) + Send + Sync + '_ {
    move |message: &str| {
        log.lock()
            .expect("the warning log")
            .push(message.to_owned());
    }
}

fn warnings(log: &Mutex<Vec<String>>) -> Vec<String> {
    log.lock().expect("the warning log").clone()
}

fn sibling_of(previous: &CliTokenData) -> CliTokenData {
    let mut sibling = previous.clone();
    sibling.key = Some(secret("sk-sibling"));
    sibling.refresh_token = Some(secret("refresh-sibling"));
    sibling.expires_at = Some(9_000_000.0_f64);
    sibling
}

fn frozen_clock() -> FixedWallClock {
    FixedWallClock(NOW)
}

#[derive(Clone, Copy)]
enum IdentityField {
    BaseUrl,
    TokenEndpoint,
    Resource,
    UserId,
    TeamId,
}

fn change_identity(field: IdentityField, record: &mut CliTokenData) {
    match field {
        IdentityField::BaseUrl => record.base_url = OTHER_SERVER.to_owned(),
        IdentityField::TokenEndpoint => {
            record.token_endpoint = Some(String::from("https://gateway.example/other-token"));
        }
        IdentityField::Resource => {
            record.resource = Some(String::from("https://gateway.example/other"));
        }
        IdentityField::UserId => record.user_id = String::from("user-2"),
        IdentityField::TeamId => record.team_id = Some(String::from("team-2")),
    }
}

#[tokio::test]
async fn renewal_saver_stores_the_record_in_the_vault_and_the_file() {
    let files = test_files();
    let mut vault = FakeVault::working();
    let log = Mutex::new(Vec::new());
    let warn = warn_sink(&log);
    let mut renewal = RenewalVault {
        store: CredentialStore {
            files: &files,
            vault: &mut vault,
        },
        warn: &warn,
        token_path: files.path(),
    };

    let outcome = renewal.save(record()).await;

    assert!(matches!(outcome, SecretSave::SecretStored));
    assert!(warnings(&log).is_empty());
    assert_eq!(vault.writes.len(), 1);
    assert!(files.stored().is_some());
}

#[tokio::test]
async fn renewal_saver_warns_when_no_store_keeps_the_credential() {
    let mut files = test_files();
    files.refusals.push(Refusal::Write);
    let mut vault = FakeVault::unavailable(KeyringUnusable::KeyringUnreachable);
    let log = Mutex::new(Vec::new());
    let warn = warn_sink(&log);
    let mut renewal = RenewalVault {
        store: CredentialStore {
            files: &files,
            vault: &mut vault,
        },
        warn: &warn,
        token_path: files.path(),
    };

    let outcome = renewal.save(record()).await;

    assert!(matches!(outcome, SecretSave::CredentialNotSaved(_)));
    let messages = warnings(&log);
    assert_eq!(messages.len(), 1);
    assert!(messages[0].starts_with("Signed in, but the credential could not be saved to"));
}

#[tokio::test]
async fn renewal_saver_warns_when_the_file_cannot_record_the_secret() {
    let mut files = test_files();
    files.refusals.push(Refusal::Commit);
    let mut vault = FakeVault::working();
    let log = Mutex::new(Vec::new());
    let warn = warn_sink(&log);
    let mut renewal = RenewalVault {
        store: CredentialStore {
            files: &files,
            vault: &mut vault,
        },
        warn: &warn,
        token_path: files.path(),
    };

    let outcome = renewal.save(record()).await;

    assert!(matches!(outcome, SecretSave::CredentialNotRecorded(_)));
    let messages = warnings(&log);
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("could not be replaced"));
}

#[tokio::test]
async fn renewal_reader_returns_the_record_the_vault_keeps() {
    let files = test_files();
    let mut vault = FakeVault::working();
    assert!(matches!(
        save_cli_token(&files, &mut vault, record()).await,
        SecretSave::SecretStored
    ));
    let log = Mutex::new(Vec::new());
    let warn = warn_sink(&log);
    let mut renewal = RenewalVault {
        store: CredentialStore {
            files: &files,
            vault: &mut vault,
        },
        warn: &warn,
        token_path: files.path(),
    };

    let reloaded = renewal.reload().await.expect("the stored record");

    assert_eq!(
        reloaded.key.as_ref().expect("a key").expose_secret(),
        "sk-secret"
    );
    assert_eq!(reloaded.base_url, SERVER);
}

#[tokio::test]
async fn renewal_reader_returns_none_without_a_stored_login() {
    let files = test_files();
    let mut vault = FakeVault::working();
    let log = Mutex::new(Vec::new());
    let warn = warn_sink(&log);
    let mut renewal = RenewalVault {
        store: CredentialStore {
            files: &files,
            vault: &mut vault,
        },
        warn: &warn,
        token_path: files.path(),
    };

    assert!(renewal.reload().await.is_none());
}

async fn successful_renewal(
    record: &CliTokenData,
) -> (
    Option<SecretString>,
    ScriptedStore,
    ScriptedHttp,
    Vec<String>,
) {
    let mut store = ScriptedStore::default();
    let http = scripted_http(vec![Reply::Response(response(
        200,
        &token_body("sk-renewed", "refresh-2", 3_600),
    ))]);
    let log = Mutex::new(Vec::new());
    let clock = frozen_clock();
    let warn = warn_sink(&log);
    let key = fresh_api_key(FreshApiKeyRequest {
        record,
        store: &mut store,
        http: &http,
        clock: &clock,
        warn: &warn,
    })
    .await;
    (key, store, http, warnings(&log))
}

#[tokio::test]
async fn fresh_api_key_returns_none_without_a_key() {
    let mut keyless = record_expiring_at(1_000_100.0_f64);
    keyless.key = None;
    let (key, store, http, log) = successful_renewal(&keyless).await;

    assert!(key.is_none());
    assert!(store.saved.is_empty());
    assert!(http.requests().is_empty());
    assert!(log.is_empty());
}

#[tokio::test]
async fn fresh_api_key_returns_none_for_an_empty_key() {
    let mut empty = record_expiring_at(1_000_100.0_f64);
    empty.key = Some(SecretString::from(String::new()));
    let (key, store, http, _log) = successful_renewal(&empty).await;

    assert!(key.is_none());
    assert!(store.saved.is_empty());
    assert!(http.requests().is_empty());
}

#[tokio::test]
async fn fresh_api_key_returns_a_key_without_an_expiry_as_stored() {
    let plain = record();
    let (key, store, http, log) = successful_renewal(&plain).await;

    assert_eq!(key.expect("the stored key").expose_secret(), "sk-secret");
    assert!(store.saved.is_empty());
    assert!(http.requests().is_empty());
    assert!(log.is_empty());
}

#[rstest]
#[case::inside_the_buffer(1_000_360.0_f64, true)]
#[case::outside_the_buffer(1_000_361.0_f64, false)]
#[tokio::test]
async fn fresh_api_key_renews_exactly_at_the_freshness_buffer(
    #[case] expires_at: f64,
    #[case] renews: bool,
) {
    let expiring = record_expiring_at(expires_at);
    let (key, store, http, _log) = successful_renewal(&expiring).await;
    let expected_key = if renews { "sk-renewed" } else { "sk-secret" };

    assert_eq!(key.expect("a key").expose_secret(), expected_key);
    assert_eq!(store.saved.len(), usize::from(u8::from(renews)));
    assert_eq!(http.requests().len(), usize::from(u8::from(renews)));
}

#[tokio::test]
async fn fresh_api_key_saves_the_rotated_credential_before_returning_the_new_key() {
    let (key, store, _http, _log) = successful_renewal(&record_expiring_at(1_000_100.0_f64)).await;

    assert_eq!(key.expect("the renewed key").expose_secret(), "sk-renewed");
    assert_eq!(store.saved.len(), 1);
    let saved = &store.saved[0];
    assert_eq!(
        saved.key.as_ref().expect("a key").expose_secret(),
        "sk-renewed"
    );
    assert_eq!(
        saved
            .refresh_token
            .as_ref()
            .expect("a refresh")
            .expose_secret(),
        "refresh-2"
    );
    assert_eq!(
        saved.expires_at.map(f64::to_bits),
        Some(1_003_600.0_f64.to_bits())
    );
    assert_eq!(saved.timestamp.to_bits(), 1_000_000.0_f64.to_bits());
    assert_eq!(saved.user_id, "cli-user");
}

#[tokio::test]
async fn fresh_api_key_posts_the_refresh_grant_to_the_stored_endpoint() {
    let (_key, _store, http, _log) = successful_renewal(&record_expiring_at(1_000_100.0_f64)).await;
    let requests = http.requests();

    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, TOKEN_ENDPOINT);
    assert_eq!(requests[0].timeout, 15);
    assert!(!requests[0].follow_redirects);
    let fields = form_fields(&requests[0].body);
    assert_eq!(form_value(&fields, "grant_type"), "refresh_token");
    assert_eq!(form_value(&fields, "refresh_token"), "refresh-secret");
    assert_eq!(form_value(&fields, "client_id"), "client-1");
    assert_eq!(form_value(&fields, "resource"), RESOURCE);
}

#[tokio::test]
async fn fresh_api_key_warns_when_a_refresh_fails_without_a_sibling() {
    let mut store = ScriptedStore::default();
    let http = scripted_http(vec![Reply::Response(response(
        400,
        r#"{"error":"invalid_grant"}"#,
    ))]);
    let clock = frozen_clock();
    let log = Mutex::new(Vec::new());
    let warn = warn_sink(&log);

    let key = fresh_api_key(FreshApiKeyRequest {
        record: &record_expiring_at(1_000_100.0_f64),
        store: &mut store,
        http: &http,
        clock: &clock,
        warn: &warn,
    })
    .await;

    assert_eq!(
        key.expect("the still valid key").expose_secret(),
        "sk-secret"
    );
    assert!(store.saved.is_empty());
    assert_eq!(
        warnings(&log),
        vec![String::from("Could not renew the key: invalid_grant")]
    );
}

#[tokio::test]
async fn fresh_api_key_returns_none_when_the_failed_key_is_expired() {
    let mut store = ScriptedStore::default();
    let http = scripted_http(vec![Reply::Transport]);
    let clock = frozen_clock();
    let log = Mutex::new(Vec::new());
    let warn = warn_sink(&log);

    let key = fresh_api_key(FreshApiKeyRequest {
        record: &record_expiring_at(999_000.0_f64),
        store: &mut store,
        http: &http,
        clock: &clock,
        warn: &warn,
    })
    .await;

    assert!(key.is_none());
    assert_eq!(warnings(&log).len(), 1);
}

#[tokio::test]
async fn fresh_api_key_uses_a_sibling_key_without_warning() {
    let files = test_files();
    let mut vault = FakeVault::working();
    let previous = record_expiring_at(1_000_100.0_f64);
    assert!(matches!(
        save_cli_token(&files, &mut vault, sibling_of(&previous)).await,
        SecretSave::SecretStored
    ));
    let http = scripted_http(vec![Reply::Transport]);
    let clock = frozen_clock();
    let log = Mutex::new(Vec::new());
    let warn = warn_sink(&log);
    let mut renewal = RenewalVault {
        store: CredentialStore {
            files: &files,
            vault: &mut vault,
        },
        warn: &warn,
        token_path: files.path(),
    };

    let key = fresh_api_key(FreshApiKeyRequest {
        record: &previous,
        store: &mut renewal,
        http: &http,
        clock: &clock,
        warn: &warn,
    })
    .await;

    assert_eq!(key.expect("the sibling key").expose_secret(), "sk-sibling");
    assert!(warnings(&log).is_empty());
}

#[tokio::test]
async fn fresh_api_key_warns_when_the_reloaded_record_is_the_same_credential() {
    let previous = record_expiring_at(1_000_100.0_f64);
    let mut store = scripted_store(Some(previous.clone()));
    let http = scripted_http(vec![Reply::Transport]);
    let clock = frozen_clock();
    let log = Mutex::new(Vec::new());
    let warn = warn_sink(&log);

    let key = fresh_api_key(FreshApiKeyRequest {
        record: &previous,
        store: &mut store,
        http: &http,
        clock: &clock,
        warn: &warn,
    })
    .await;

    assert_eq!(key.expect("the old key").expose_secret(), "sk-secret");
    assert_eq!(warnings(&log).len(), 1);
}

#[tokio::test]
async fn fresh_api_key_warns_when_the_reloaded_record_has_another_identity() {
    let previous = record_expiring_at(1_000_100.0_f64);
    let mut foreign = sibling_of(&previous);
    change_identity(IdentityField::UserId, &mut foreign);
    let mut store = scripted_store(Some(foreign));
    let http = scripted_http(vec![Reply::Transport]);
    let clock = frozen_clock();
    let log = Mutex::new(Vec::new());
    let warn = warn_sink(&log);

    let key = fresh_api_key(FreshApiKeyRequest {
        record: &previous,
        store: &mut store,
        http: &http,
        clock: &clock,
        warn: &warn,
    })
    .await;

    assert_eq!(key.expect("the old key").expose_secret(), "sk-secret");
    assert_eq!(warnings(&log).len(), 1);
}

#[tokio::test]
async fn fresh_api_key_skips_the_refresh_without_refresh_inputs() {
    let mut incomplete = record_expiring_at(1_000_100.0_f64);
    incomplete.client_id = None;
    let (key, store, http, log) = successful_renewal(&incomplete).await;

    assert_eq!(
        key.expect("the still valid key").expose_secret(),
        "sk-secret"
    );
    assert!(store.saved.is_empty());
    assert!(http.requests().is_empty());
    assert!(log.is_empty());
}

#[test]
fn sibling_rotation_returns_the_key_the_other_process_saved() {
    let previous = record_expiring_at(5_000.0_f64);
    let key = key_rotated_by_a_sibling(Some(&sibling_of(&previous)), &previous, SIBLING_NOW)
        .expect("the sibling key");

    assert_eq!(key.expose_secret(), "sk-sibling");
}

#[rstest]
#[case::base_url(IdentityField::BaseUrl)]
#[case::token_endpoint(IdentityField::TokenEndpoint)]
#[case::resource(IdentityField::Resource)]
#[case::user_id(IdentityField::UserId)]
#[case::team_id(IdentityField::TeamId)]
fn sibling_rotation_rejects_another_identity(#[case] field: IdentityField) {
    let previous = record_expiring_at(5_000.0_f64);
    let mut reloaded = sibling_of(&previous);
    change_identity(field, &mut reloaded);

    assert!(key_rotated_by_a_sibling(Some(&reloaded), &previous, SIBLING_NOW).is_none());
}

#[test]
fn sibling_rotation_rejects_the_same_refresh_token() {
    let previous = record_expiring_at(5_000.0_f64);
    let mut reloaded = sibling_of(&previous);
    reloaded.refresh_token = previous.refresh_token.clone();

    assert!(key_rotated_by_a_sibling(Some(&reloaded), &previous, SIBLING_NOW).is_none());
}

#[rstest]
#[case::at_the_expiry(9_000_000.0_f64)]
#[case::past_the_expiry(9_000_001.0_f64)]
fn sibling_rotation_rejects_an_expired_record(#[case] now_seconds: f64) {
    let previous = record_expiring_at(5_000.0_f64);

    assert!(
        key_rotated_by_a_sibling(Some(&sibling_of(&previous)), &previous, now_seconds).is_none()
    );
}

#[test]
fn sibling_rotation_rejects_a_record_without_an_expiry() {
    let previous = record_expiring_at(5_000.0_f64);
    let mut reloaded = sibling_of(&previous);
    reloaded.expires_at = None;

    assert!(key_rotated_by_a_sibling(Some(&reloaded), &previous, SIBLING_NOW).is_none());
}

#[rstest]
#[case::missing(None)]
#[case::empty(Some(""))]
fn sibling_rotation_rejects_a_record_without_a_usable_key(#[case] key: Option<&str>) {
    let previous = record_expiring_at(5_000.0_f64);
    let mut reloaded = sibling_of(&previous);
    reloaded.key = key.map(secret);

    assert!(key_rotated_by_a_sibling(Some(&reloaded), &previous, SIBLING_NOW).is_none());
}

#[test]
fn sibling_rotation_rejects_no_reloaded_record() {
    assert!(
        key_rotated_by_a_sibling(None, &record_expiring_at(5_000.0_f64), SIBLING_NOW).is_none()
    );
}

#[test]
fn refresh_inputs_returns_the_five_values() {
    let inputs = refresh_inputs(&record()).expect("the refresh inputs");

    assert_eq!(inputs.token_endpoint, TOKEN_ENDPOINT);
    assert_eq!(inputs.revocation_endpoint, REVOCATION_ENDPOINT);
    assert_eq!(inputs.resource, RESOURCE);
    assert_eq!(inputs.client_id, "client-1");
    assert_eq!(inputs.refresh_token.expose_secret(), "refresh-secret");
}

#[derive(Clone, Copy)]
enum RefreshField {
    TokenEndpoint,
    RevocationEndpoint,
    Resource,
    ClientId,
    RefreshToken,
}

fn clear_field(field: RefreshField, record: &mut CliTokenData) {
    match field {
        RefreshField::TokenEndpoint => record.token_endpoint = None,
        RefreshField::RevocationEndpoint => record.revocation_endpoint = None,
        RefreshField::Resource => record.resource = None,
        RefreshField::ClientId => record.client_id = None,
        RefreshField::RefreshToken => record.refresh_token = None,
    }
}

fn empty_field(field: RefreshField, record: &mut CliTokenData) {
    match field {
        RefreshField::TokenEndpoint => record.token_endpoint = Some(String::new()),
        RefreshField::RevocationEndpoint => record.revocation_endpoint = Some(String::new()),
        RefreshField::Resource => record.resource = Some(String::new()),
        RefreshField::ClientId => record.client_id = Some(String::new()),
        RefreshField::RefreshToken => record.refresh_token = Some(secret("")),
    }
}

#[rstest]
#[case::token_endpoint(RefreshField::TokenEndpoint)]
#[case::revocation_endpoint(RefreshField::RevocationEndpoint)]
#[case::resource(RefreshField::Resource)]
#[case::client_id(RefreshField::ClientId)]
#[case::refresh_token(RefreshField::RefreshToken)]
fn refresh_inputs_rejects_a_missing_field(#[case] field: RefreshField) {
    let mut missing = record();
    clear_field(field, &mut missing);

    assert!(refresh_inputs(&missing).is_none());
}

#[rstest]
#[case::token_endpoint(RefreshField::TokenEndpoint)]
#[case::revocation_endpoint(RefreshField::RevocationEndpoint)]
#[case::resource(RefreshField::Resource)]
#[case::client_id(RefreshField::ClientId)]
#[case::refresh_token(RefreshField::RefreshToken)]
fn refresh_inputs_rejects_an_empty_field(#[case] field: RefreshField) {
    let mut empty = record();
    empty_field(field, &mut empty);

    assert!(refresh_inputs(&empty).is_none());
}
