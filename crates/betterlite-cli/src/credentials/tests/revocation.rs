use std::sync::{Arc, Mutex};

use secrecy::ExposeSecret as _;

use crate::credential_lifecycle::load::load_cli_token;
use crate::credential_lifecycle::outcome::SecretSave;
use crate::credential_lifecycle::save::save_cli_token;
use crate::credentials::revocation::{
    ReplaceStoredTokenRequest, replace_stored_token, revoke_stored_credential,
};
use crate::keyring::KeyringUnusable;
use crate::pkce_login::models::PkceFailure;
use crate::pkce_login::revoke::RevocationUnavailableOrFailure;

use super::support::{
    FakeVault, REVOCATION_ENDPOINT, Refusal, Reply, SERVER, TestFiles, form_fields, form_value,
    record, redirect, response, scripted_http, secret, test_files,
};

fn warn_sink(log: &Mutex<Vec<String>>) -> impl Fn(&str) + Send + Sync + '_ {
    move |message: &str| {
        log.lock().expect("the echo log").push(message.to_owned());
    }
}

#[test]
fn revocation_skips_a_record_without_a_refresh_token() {
    let mut without_refresh = record();
    without_refresh.refresh_token = None;
    let http = scripted_http(Vec::new());

    assert!(revoke_stored_credential(&without_refresh, &http).is_none());
    assert!(http.requests().is_empty());
}

#[test]
fn revocation_skips_a_record_that_cannot_refresh() {
    let mut without_token_endpoint = record();
    without_token_endpoint.token_endpoint = None;
    let http = scripted_http(Vec::new());

    assert!(revoke_stored_credential(&without_token_endpoint, &http).is_none());
    assert!(http.requests().is_empty());
}

#[test]
fn revocation_posts_the_stored_refresh_token_to_the_stored_endpoint() {
    let http = scripted_http(vec![Reply::Response(response(200, ""))]);

    let outcome = revoke_stored_credential(&record(), &http);

    assert!(outcome.is_none());
    let requests = http.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, REVOCATION_ENDPOINT);
    assert_eq!(requests[0].timeout, 15);
    assert!(!requests[0].follow_redirects);
    let fields = form_fields(&requests[0].body);
    assert_eq!(form_value(&fields, "token"), "refresh-secret");
    assert_eq!(form_value(&fields, "token_type_hint"), "refresh_token");
    assert_eq!(form_value(&fields, "client_id"), "client-1");
}

#[test]
fn revocation_reports_a_temporarily_unavailable_endpoint() {
    let http = scripted_http(vec![Reply::Response(response(
        503,
        r#"{"error":"temporarily_unavailable"}"#,
    ))]);

    let outcome = revoke_stored_credential(&record(), &http).expect("the transient failure");

    let RevocationUnavailableOrFailure::Unavailable(unavailable) = outcome else {
        panic!("expected the transient failure");
    };
    assert_eq!(unavailable.reason, "temporarily_unavailable");
}

#[test]
fn revocation_reports_a_permanent_failure() {
    let http = scripted_http(vec![Reply::Response(response(
        400,
        r#"{"error":"invalid_client"}"#,
    ))]);

    let outcome = revoke_stored_credential(&record(), &http).expect("the failure");

    let RevocationUnavailableOrFailure::Failure(PkceFailure::Protocol(message)) = outcome else {
        panic!("expected the permanent failure");
    };
    assert_eq!(message, "invalid_client");
}

#[test]
fn revocation_refuses_to_follow_a_redirect() {
    let http = scripted_http(vec![Reply::Response(redirect("https://elsewhere.example"))]);

    let outcome = revoke_stored_credential(&record(), &http).expect("the failure");

    let RevocationUnavailableOrFailure::Failure(PkceFailure::Protocol(message)) = outcome else {
        panic!("expected the refused redirect");
    };
    assert!(message.contains("refusing to follow"));
}

#[test]
fn revocation_reports_a_failed_transport() {
    let http = scripted_http(vec![Reply::Transport]);

    let outcome = revoke_stored_credential(&record(), &http).expect("the failure");

    assert!(matches!(
        outcome,
        RevocationUnavailableOrFailure::Failure(PkceFailure::Protocol(_message))
    ));
}

async fn stored_previous(files: &TestFiles, vault: &mut FakeVault) {
    assert!(matches!(
        save_cli_token(files, vault, record()).await,
        SecretSave::SecretStored
    ));
}

#[tokio::test]
async fn replacement_saves_the_new_login_and_revokes_only_the_previous_token() {
    let files = test_files();
    let mut vault = FakeVault::working();
    stored_previous(&files, &mut vault).await;
    vault.clear_events();
    let http = Arc::new(
        scripted_http(vec![Reply::Response(response(200, ""))])
            .with_events(Arc::clone(&vault.events)),
    );
    let log = Mutex::new(Vec::new());
    let echo = warn_sink(&log);
    let mut replacement = record();
    replacement.refresh_token = Some(secret("refresh-new"));

    let outcome = replace_stored_token(ReplaceStoredTokenRequest {
        record: replacement,
        files: &files,
        vault: &mut vault,
        http: Arc::clone(&http),
        echo: &echo,
    })
    .await;

    assert!(matches!(outcome, SecretSave::SecretStored));
    let fields = form_fields(&http.requests()[0].body);
    assert_eq!(form_value(&fields, "token"), "refresh-secret");
    assert_eq!(
        vault.events(),
        vec![
            String::from("vault-save"),
            format!("http {REVOCATION_ENDPOINT}")
        ]
    );
    assert!(log.lock().expect("the echo log").is_empty());
}

#[tokio::test]
async fn replacement_skips_revocation_when_nothing_was_stored_before() {
    let files = test_files();
    let mut vault = FakeVault::working();
    let http = Arc::new(scripted_http(Vec::new()));
    let log = Mutex::new(Vec::new());
    let echo = warn_sink(&log);

    let outcome = replace_stored_token(ReplaceStoredTokenRequest {
        record: record(),
        files: &files,
        vault: &mut vault,
        http: Arc::clone(&http),
        echo: &echo,
    })
    .await;

    assert!(matches!(outcome, SecretSave::SecretStored));
    assert!(http.requests().is_empty());
}

#[tokio::test]
async fn replacement_skips_revocation_when_the_new_login_could_not_be_saved() {
    let mut files = test_files();
    let mut vault = FakeVault::working();
    stored_previous(&files, &mut vault).await;
    files.refusals.push(Refusal::Write);
    let mut unavailable = FakeVault::unavailable(KeyringUnusable::KeyringUnreachable);
    let http = Arc::new(scripted_http(Vec::new()));
    let log = Mutex::new(Vec::new());
    let echo = warn_sink(&log);

    let outcome = replace_stored_token(ReplaceStoredTokenRequest {
        record: record(),
        files: &files,
        vault: &mut unavailable,
        http: Arc::clone(&http),
        echo: &echo,
    })
    .await;

    assert!(matches!(outcome, SecretSave::CredentialNotSaved(_)));
    assert!(http.requests().is_empty());
}

#[tokio::test]
async fn replacement_still_revokes_when_the_file_could_not_record_the_secret() {
    let mut files = test_files();
    let mut vault = FakeVault::working();
    stored_previous(&files, &mut vault).await;
    files.refusals.push(Refusal::Commit);
    let http = Arc::new(scripted_http(vec![Reply::Response(response(200, ""))]));
    let log = Mutex::new(Vec::new());
    let echo = warn_sink(&log);

    let outcome = replace_stored_token(ReplaceStoredTokenRequest {
        record: record(),
        files: &files,
        vault: &mut vault,
        http: Arc::clone(&http),
        echo: &echo,
    })
    .await;

    assert!(matches!(outcome, SecretSave::CredentialNotRecorded(_)));
    assert_eq!(http.requests().len(), 1);
}

#[tokio::test]
async fn replacement_warns_when_the_previous_token_cannot_be_revoked() {
    let files = test_files();
    let mut vault = FakeVault::working();
    stored_previous(&files, &mut vault).await;
    let http = Arc::new(scripted_http(vec![Reply::Response(response(
        503,
        r#"{"error":"temporarily_unavailable"}"#,
    ))]));
    let log = Mutex::new(Vec::new());
    let echo = warn_sink(&log);

    let outcome = replace_stored_token(ReplaceStoredTokenRequest {
        record: record(),
        files: &files,
        vault: &mut vault,
        http: Arc::clone(&http),
        echo: &echo,
    })
    .await;

    assert!(matches!(outcome, SecretSave::SecretStored));
    let messages = log.lock().expect("the echo log").clone();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("temporarily_unavailable"));
    assert!(messages[0].ends_with("it expires on its own."));
}

#[tokio::test]
async fn replacement_stores_the_new_record_for_the_next_command() {
    let files = test_files();
    let mut vault = FakeVault::working();
    stored_previous(&files, &mut vault).await;
    let http = Arc::new(
        scripted_http(vec![Reply::Response(response(200, ""))])
            .with_events(Arc::clone(&vault.events)),
    );
    let log = Mutex::new(Vec::new());
    let echo = warn_sink(&log);
    let mut replacement = record();
    replacement.key = Some(secret("sk-new"));
    replacement.refresh_token = Some(secret("refresh-new"));

    assert!(matches!(
        replace_stored_token(ReplaceStoredTokenRequest {
            record: replacement,
            files: &files,
            vault: &mut vault,
            http: Arc::clone(&http),
            echo: &echo,
        })
        .await,
        SecretSave::SecretStored
    ));
    let loaded = load_cli_token(&files, &mut vault)
        .await
        .expect("the new record");

    assert_eq!(
        loaded.key.as_ref().expect("a key").expose_secret(),
        "sk-new"
    );
    assert_eq!(
        loaded
            .refresh_token
            .as_ref()
            .expect("a refresh")
            .expose_secret(),
        "refresh-new"
    );
    assert_eq!(loaded.base_url, SERVER);
}
