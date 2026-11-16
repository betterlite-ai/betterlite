#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::daily_activity_contracts::KeyMetadataRow;
use betterlite_db::postgres_store::session::PostgresStore;
use tracing::subscriber::set_default;

use super::support::{
    DIGEST, at, capture_logs, close, database, insert_daily_spend, insert_spend_log, insert_user,
    keys, moment, reads, recover, window,
};

#[tokio::test]
async fn spend_owner_fills_user_id_without_filling_a_team() {
    let database = database().await;
    insert_daily_spend(&database.store, DIGEST, "owner-1", "2026-01-05").await;
    insert_user(
        &database.store,
        "owner-1",
        Some("owner@example.com"),
        &["team-o"],
    )
    .await;
    let recovered = recover(
        &reads(&database.store),
        BTreeMap::new(),
        &keys(&[DIGEST]),
        None,
    )
    .await;
    assert_eq!(
        recovered.get(DIGEST),
        Some(&KeyMetadataRow {
            api_key: DIGEST.to_owned(),
            key_alias: None,
            team_id: None,
            user_id: Some("owner-1".to_owned()),
            user_email: Some("owner@example.com".to_owned()),
            key_exists: false,
            tags: Vec::new(),
        })
    );
    close(database).await;
}

#[tokio::test]
async fn spend_owner_merges_into_spend_log_evidence() {
    let database = database().await;
    insert_spend_log(
        &database.store,
        DIGEST,
        moment(2026, 1, 5, 10, 0),
        "spend-alias",
        None,
    )
    .await;
    insert_daily_spend(&database.store, DIGEST, "owner-1", "2026-01-05").await;
    insert_user(
        &database.store,
        "owner-1",
        Some("owner@example.com"),
        &["team-o"],
    )
    .await;
    let recovered = recover(
        &reads(&database.store),
        BTreeMap::new(),
        &keys(&[DIGEST]),
        Some(window(at(2026, 1, 4, 0, 0), at(2026, 1, 7, 0, 0))),
    )
    .await;
    assert_eq!(
        recovered.get(DIGEST),
        Some(&KeyMetadataRow {
            api_key: DIGEST.to_owned(),
            key_alias: Some("spend-alias".to_owned()),
            team_id: None,
            user_id: Some("owner-1".to_owned()),
            user_email: Some("owner@example.com".to_owned()),
            key_exists: false,
            tags: Vec::new(),
        })
    );
    close(database).await;
}

#[tokio::test]
async fn spend_owner_skips_a_key_confirmed_to_exist() {
    let database = database().await;
    insert_daily_spend(&database.store, "sk-live", "owner-1", "2026-01-05").await;
    insert_user(&database.store, "owner-1", None, &[]).await;
    let resolved = BTreeMap::from([(
        "sk-live".to_owned(),
        KeyMetadataRow {
            api_key: "sk-live".to_owned(),
            key_alias: Some("live-alias".to_owned()),
            team_id: None,
            user_id: None,
            user_email: None,
            key_exists: true,
            tags: Vec::new(),
        },
    )]);
    let recovered = recover(
        &reads(&database.store),
        resolved.clone(),
        &keys(&["sk-live"]),
        None,
    )
    .await;
    assert_eq!(recovered, resolved);
    close(database).await;
}

#[tokio::test]
async fn spend_owner_skips_a_key_that_already_has_a_user() {
    let database = database().await;
    insert_daily_spend(&database.store, "sk-owned", "owner-1", "2026-01-05").await;
    insert_user(&database.store, "known", Some("known@example.com"), &[]).await;
    let resolved = BTreeMap::from([(
        "sk-owned".to_owned(),
        KeyMetadataRow {
            api_key: "sk-owned".to_owned(),
            key_alias: Some("owned-alias".to_owned()),
            team_id: None,
            user_id: Some("known".to_owned()),
            user_email: None,
            key_exists: false,
            tags: Vec::new(),
        },
    )]);
    let recovered = recover(
        &reads(&database.store),
        resolved,
        &keys(&["sk-owned"]),
        None,
    )
    .await;
    assert_eq!(
        recovered.get("sk-owned"),
        Some(&KeyMetadataRow {
            api_key: "sk-owned".to_owned(),
            key_alias: Some("owned-alias".to_owned()),
            team_id: None,
            user_id: Some("known".to_owned()),
            user_email: Some("known@example.com".to_owned()),
            key_exists: false,
            tags: Vec::new(),
        })
    );
    close(database).await;
}

#[tokio::test]
async fn spend_owner_treats_an_empty_user_id_as_missing() {
    let database = database().await;
    insert_daily_spend(&database.store, DIGEST, "owner-1", "2026-01-05").await;
    insert_user(&database.store, "owner-1", Some("owner@example.com"), &[]).await;
    let resolved = BTreeMap::from([(
        DIGEST.to_owned(),
        KeyMetadataRow {
            api_key: DIGEST.to_owned(),
            key_alias: None,
            team_id: None,
            user_id: Some(String::new()),
            user_email: None,
            key_exists: false,
            tags: Vec::new(),
        },
    )]);
    let recovered = recover(&reads(&database.store), resolved, &keys(&[DIGEST]), None).await;
    assert_eq!(
        recovered.get(DIGEST),
        Some(&KeyMetadataRow {
            api_key: DIGEST.to_owned(),
            key_alias: None,
            team_id: None,
            user_id: Some("owner-1".to_owned()),
            user_email: Some("owner@example.com".to_owned()),
            key_exists: false,
            tags: Vec::new(),
        })
    );
    close(database).await;
}

#[tokio::test]
async fn spend_owner_requires_a_unanimous_owner() {
    let database = database().await;
    insert_daily_spend(&database.store, DIGEST, "user-a", "2026-01-05").await;
    insert_daily_spend(&database.store, DIGEST, "user-b", "2026-01-06").await;
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

#[tokio::test]
async fn spend_owner_uses_one_owner_repeated_across_rows() {
    let database = database().await;
    insert_daily_spend(&database.store, DIGEST, "owner-1", "2026-01-05").await;
    insert_daily_spend(&database.store, DIGEST, "owner-1", "2026-01-06").await;
    insert_user(&database.store, "owner-1", None, &[]).await;
    let recovered = recover(
        &reads(&database.store),
        BTreeMap::new(),
        &keys(&[DIGEST]),
        None,
    )
    .await;
    let row = &recovered[DIGEST];
    assert_eq!(row.user_id.as_deref(), Some("owner-1"));
    assert!(row.user_email.is_none());
    close(database).await;
}

#[tokio::test]
async fn spend_owner_ignores_an_empty_owner_row() {
    let database = database().await;
    insert_daily_spend(&database.store, DIGEST, "", "2026-01-05").await;
    insert_daily_spend(&database.store, DIGEST, "owner-1", "2026-01-06").await;
    insert_user(&database.store, "owner-1", None, &[]).await;
    let recovered = recover(
        &reads(&database.store),
        BTreeMap::new(),
        &keys(&[DIGEST]),
        None,
    )
    .await;
    let row = &recovered[DIGEST];
    assert_eq!(row.user_id.as_deref(), Some("owner-1"));
    close(database).await;
}

#[tokio::test]
async fn spend_owner_leaves_a_key_with_a_user_and_email_alone_without_a_query() {
    let store = PostgresStore { pool: None };
    let resolved = BTreeMap::from([(
        DIGEST.to_owned(),
        KeyMetadataRow {
            api_key: DIGEST.to_owned(),
            key_alias: None,
            team_id: None,
            user_id: Some("known".to_owned()),
            user_email: Some("known@example.com".to_owned()),
            key_exists: false,
            tags: Vec::new(),
        },
    )]);
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    let recovered = recover(&reads(&store), resolved.clone(), &keys(&[DIGEST]), None).await;
    drop(guard);
    assert_eq!(recovered, resolved);
    assert!(logs.contents().is_empty());
}
