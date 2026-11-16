#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::daily_activity_contracts::KeyMetadataRow;
use betterlite_db::postgres_store::session::PostgresStore;
use tracing::subscriber::set_default;

use super::support::{
    DIGEST, at, capture_logs, close, database, insert_spend_log, keys, moment, reads, recover,
    window,
};

#[tokio::test]
async fn empty_recovery_looks_nothing_up_for_an_empty_key_set() {
    let store = PostgresStore { pool: None };
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    let recovered = recover(
        &reads(&store),
        BTreeMap::new(),
        &keys(&[]),
        Some(window(at(2026, 1, 4, 0, 0), at(2026, 1, 7, 0, 0))),
    )
    .await;
    drop(guard);
    assert!(recovered.is_empty());
    assert!(logs.contents().is_empty());
}

#[tokio::test]
async fn empty_recovery_keeps_resolved_rows_without_looking_anything_up() {
    let store = PostgresStore { pool: None };
    let resolved = BTreeMap::from([(
        "sk-resolved".to_owned(),
        KeyMetadataRow {
            api_key: "sk-resolved".to_owned(),
            key_alias: Some("resolved-alias".to_owned()),
            team_id: Some("resolved-team".to_owned()),
            user_id: Some("resolved-user".to_owned()),
            user_email: Some("resolved@example.com".to_owned()),
            key_exists: true,
            tags: vec!["prod".to_owned()],
        },
    )]);
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    let recovered = recover(
        &reads(&store),
        resolved.clone(),
        &keys(&["sk-resolved"]),
        Some(window(at(2026, 1, 4, 0, 0), at(2026, 1, 7, 0, 0))),
    )
    .await;
    drop(guard);
    assert_eq!(recovered, resolved);
    assert!(logs.contents().is_empty());
}

#[tokio::test]
async fn empty_recovery_without_a_window_skips_spend_logs() {
    let database = database().await;
    insert_spend_log(
        &database.store,
        DIGEST,
        moment(2026, 1, 5, 10, 0),
        "spend-alias",
        None,
    )
    .await;
    let recovered = recover(
        &reads(&database.store),
        BTreeMap::new(),
        &keys(&[DIGEST]),
        None,
    )
    .await;
    assert!(recovered.is_empty());
    close(database).await;
}
