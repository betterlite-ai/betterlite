#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::daily_activity_contracts::KeyMetadataRow;
use betterlite_db::postgres_store::session::PostgresStore;
use tracing::subscriber::set_default;

use super::support::{
    DIGEST, at, capture_logs, close, database, digest_of, insert_broken_user, insert_token, keys,
    reads, recover, window,
};

#[tokio::test]
async fn recovery_error_in_the_cli_session_step_warns_and_returns_empty() {
    let database = database().await;
    insert_broken_user(&database.store, "broken").await;
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    let recovered = recover(
        &reads(&database.store),
        BTreeMap::new(),
        &keys(&["cli-session-broken"]),
        None,
    )
    .await;
    drop(guard);
    assert!(recovered.is_empty());
    let captured = logs.contents();
    assert!(captured.contains("key metadata recovery failed"));
    assert!(captured.contains("cli_session"));
    close(database).await;
}

#[tokio::test]
async fn recovery_error_keeps_later_steps_working() {
    let database = database().await;
    insert_broken_user(&database.store, "broken").await;
    insert_token(&database.store, "sk-good", "good-alias", Some("good-user")).await;
    let digest = digest_of("sk-good");
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    let recovered = recover(
        &reads(&database.store),
        BTreeMap::new(),
        &keys(&["cli-session-broken", digest.as_str()]),
        None,
    )
    .await;
    drop(guard);
    assert_eq!(
        recovered.get(digest.as_str()),
        Some(&KeyMetadataRow {
            api_key: digest.clone(),
            key_alias: Some("good-alias".to_owned()),
            team_id: None,
            user_id: Some("good-user".to_owned()),
            user_email: None,
            key_exists: false,
            tags: Vec::new(),
        })
    );
    assert!(!recovered.contains_key("cli-session-broken"));
    assert!(logs.contents().contains("cli_session"));
    close(database).await;
}

#[tokio::test]
async fn recovery_error_in_attach_keeps_metadata_without_user_details() {
    let database = database().await;
    insert_broken_user(&database.store, "broken-user").await;
    insert_token(
        &database.store,
        "sk-broken",
        "broken-alias",
        Some("broken-user"),
    )
    .await;
    let digest = digest_of("sk-broken");
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    let recovered = recover(
        &reads(&database.store),
        BTreeMap::new(),
        &keys(&[digest.as_str()]),
        None,
    )
    .await;
    drop(guard);
    assert_eq!(
        recovered.get(digest.as_str()),
        Some(&KeyMetadataRow {
            api_key: digest.clone(),
            key_alias: Some("broken-alias".to_owned()),
            team_id: None,
            user_id: Some("broken-user".to_owned()),
            user_email: None,
            key_exists: false,
            tags: Vec::new(),
        })
    );
    assert!(logs.contents().contains("user detail attachment failed"));
    close(database).await;
}

#[tokio::test]
async fn recovery_error_without_a_database_warns_and_returns_empty() {
    let store = PostgresStore { pool: None };
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    let recovered = recover(
        &reads(&store),
        BTreeMap::new(),
        &keys(&[DIGEST]),
        Some(window(at(2026, 1, 4, 0, 0), at(2026, 1, 7, 0, 0))),
    )
    .await;
    drop(guard);
    assert!(recovered.is_empty());
    assert!(logs.contents().contains("continuing without"));
}
