#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_db::daily_activity_contracts::{KeyMetadataRow, SpendLogsWindow};
use betterlite_db::daily_activity_repository::database::PostgresDailyActivityDatabase;
use betterlite_db::key_attribution::history::recovery::SpendLogHistory;
use betterlite_db::postgres_store::session::PostgresStore;
use deadpool_postgres::Client;
use tracing::subscriber::set_default;

use super::repository::daily_activity_repository;
use super::support::{
    DIGEST, at, capture_logs, client, close, database, digest_of, insert_daily_spend,
    insert_spend_log, insert_token, insert_tombstone, insert_user, keys, moment, reads, recover,
    resolved_row, window,
};

#[path = "scope.rs"]
mod scope;

#[tokio::test]
async fn recover_key_metadata_cli_session_key_resolves_alias_and_user() {
    let database = database().await;
    insert_user(
        &database.store,
        "user-1",
        Some("user@example.com"),
        &["team-a"],
    )
    .await;
    let recovered = recover(
        &reads(&database.store),
        BTreeMap::new(),
        &keys(&["cli-session-user-1"]),
        None,
    )
    .await;
    assert_eq!(
        recovered.get("cli-session-user-1"),
        Some(&KeyMetadataRow {
            api_key: "cli-session-user-1".to_owned(),
            key_alias: Some("cli-session-user-1".to_owned()),
            team_id: Some("team-a".to_owned()),
            user_id: Some("user-1".to_owned()),
            user_email: Some("user@example.com".to_owned()),
            key_exists: false,
            tags: Vec::new(),
        })
    );
    close(database).await;
}

#[tokio::test]
async fn recover_key_metadata_drops_keys_nothing_resolves() {
    let database = database().await;
    let recovered = recover(
        &reads(&database.store),
        BTreeMap::new(),
        &keys(&["cli-session-ghost", "sk-plain"]),
        None,
    )
    .await;
    assert!(recovered.is_empty());
    close(database).await;
}

#[tokio::test]
async fn recover_key_metadata_reverse_hashes_active_tokens() {
    let database = database().await;
    insert_user(
        &database.store,
        "active-user",
        Some("active@example.com"),
        &["team-active"],
    )
    .await;
    insert_token(
        &database.store,
        "sk-active",
        "active-alias",
        Some("active-user"),
    )
    .await;
    let digest = digest_of("sk-active");
    let recovered = recover(
        &reads(&database.store),
        BTreeMap::new(),
        &keys(&[digest.as_str()]),
        None,
    )
    .await;
    assert_eq!(
        recovered.get(digest.as_str()),
        Some(&KeyMetadataRow {
            api_key: digest.clone(),
            key_alias: Some("active-alias".to_owned()),
            team_id: None,
            user_id: Some("active-user".to_owned()),
            user_email: Some("active@example.com".to_owned()),
            key_exists: false,
            tags: Vec::new(),
        })
    );
    close(database).await;
}

#[tokio::test]
async fn recover_key_metadata_reverse_hashes_deleted_tokens_when_active_missing() {
    let database = database().await;
    insert_tombstone(
        &database.store,
        "d1",
        "sk-gone",
        "gone-alias",
        moment(2026, 1, 1, 0, 0),
    )
    .await;
    let digest = digest_of("sk-gone");
    let recovered = recover(
        &reads(&database.store),
        BTreeMap::new(),
        &keys(&[digest.as_str()]),
        None,
    )
    .await;
    assert_eq!(
        recovered.get(digest.as_str()),
        Some(&KeyMetadataRow {
            api_key: digest.clone(),
            key_alias: Some("gone-alias".to_owned()),
            team_id: None,
            user_id: None,
            user_email: None,
            key_exists: false,
            tags: Vec::new(),
        })
    );
    close(database).await;
}

#[tokio::test]
async fn recover_key_metadata_prefers_active_tokens_over_deleted_tokens() {
    let database = database().await;
    insert_token(&database.store, "sk-both", "active-alias", Some("u-active")).await;
    insert_tombstone(
        &database.store,
        "d2",
        "sk-both",
        "deleted-alias",
        moment(2026, 1, 2, 0, 0),
    )
    .await;
    let digest = digest_of("sk-both");
    let recovered = recover(
        &reads(&database.store),
        BTreeMap::new(),
        &keys(&[digest.as_str()]),
        None,
    )
    .await;
    let row = &recovered[digest.as_str()];
    assert_eq!(row.key_alias.as_deref(), Some("active-alias"));
    assert_eq!(row.user_id.as_deref(), Some("u-active"));
    assert!(row.user_email.is_none());
    close(database).await;
}

#[tokio::test]
async fn recover_key_metadata_token_evidence_wins_over_spend_logs() {
    let database = database().await;
    insert_token(
        &database.store,
        "sk-token",
        "token-alias",
        Some("token-user"),
    )
    .await;
    let digest = digest_of("sk-token");
    insert_spend_log(
        &database.store,
        digest.as_str(),
        moment(2026, 1, 5, 10, 0),
        "spend-alias",
        None,
    )
    .await;
    let recovered = recover(
        &reads(&database.store),
        BTreeMap::new(),
        &keys(&[digest.as_str()]),
        Some(window(at(2026, 1, 4, 0, 0), at(2026, 1, 7, 0, 0))),
    )
    .await;
    let row = &recovered[digest.as_str()];
    assert_eq!(row.key_alias.as_deref(), Some("token-alias"));
    assert_eq!(row.user_id.as_deref(), Some("token-user"));
    close(database).await;
}

#[tokio::test]
async fn recover_key_metadata_spend_logs_name_unresolved_digests() {
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
        Some(window(at(2026, 1, 4, 0, 0), at(2026, 1, 7, 0, 0))),
    )
    .await;
    assert_eq!(
        recovered.get(DIGEST),
        Some(&KeyMetadataRow {
            api_key: DIGEST.to_owned(),
            key_alias: Some("spend-alias".to_owned()),
            team_id: None,
            user_id: None,
            user_email: None,
            key_exists: false,
            tags: Vec::new(),
        })
    );
    close(database).await;
}

#[tokio::test]
async fn recover_key_metadata_merges_a_recovered_owner_into_spend_log_metadata() {
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
async fn recover_key_metadata_does_not_recover_resolved_keys_again() {
    let database = database().await;
    insert_token(
        &database.store,
        "cli-session-known",
        "token-alias",
        Some("token-user"),
    )
    .await;
    insert_user(
        &database.store,
        "known",
        Some("known@example.com"),
        &["team-k"],
    )
    .await;
    let resolved = BTreeMap::from([(
        "cli-session-known".to_owned(),
        KeyMetadataRow {
            api_key: "cli-session-known".to_owned(),
            key_alias: None,
            team_id: None,
            user_id: None,
            user_email: None,
            key_exists: true,
            tags: vec!["tag-1".to_owned()],
        },
    )]);
    let recovered = recover(
        &reads(&database.store),
        resolved.clone(),
        &keys(&["cli-session-known"]),
        None,
    )
    .await;
    assert_eq!(recovered, resolved);
    close(database).await;
}

#[tokio::test]
async fn recover_key_metadata_keeps_resolved_rows_fields_and_tags() {
    let database = database().await;
    insert_token(
        &database.store,
        "sk-live",
        "token-alias",
        Some("token-user"),
    )
    .await;
    insert_user(
        &database.store,
        "resolved-user",
        Some("resolved@example.com"),
        &[],
    )
    .await;
    let row = KeyMetadataRow {
        key_alias: Some("resolved-alias".to_owned()),
        team_id: Some("resolved-team".to_owned()),
        key_exists: true,
        tags: vec!["prod".to_owned()],
        ..resolved_row("sk-live", "resolved-user")
    };
    let resolved = BTreeMap::from([("sk-live".to_owned(), row.clone())]);
    let recovered = recover(&reads(&database.store), resolved, &keys(&["sk-live"]), None).await;
    let expected = KeyMetadataRow {
        user_email: Some("resolved@example.com".to_owned()),
        ..row
    };
    assert_eq!(recovered.get("sk-live"), Some(&expected));
    close(database).await;
}

#[tokio::test]
async fn recover_key_metadata_returns_resolved_keys_outside_the_key_set() {
    let store = PostgresStore { pool: None };
    let resolved = BTreeMap::from([(
        "sk-extra".to_owned(),
        KeyMetadataRow {
            api_key: "sk-extra".to_owned(),
            key_alias: Some("extra-alias".to_owned()),
            team_id: None,
            user_id: Some("extra-user".to_owned()),
            user_email: Some("extra@example.com".to_owned()),
            key_exists: true,
            tags: vec!["extra".to_owned()],
        },
    )]);
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    let recovered = recover(&reads(&store), resolved.clone(), &keys(&[]), None).await;
    drop(guard);
    assert_eq!(recovered, resolved);
    assert!(logs.contents().is_empty());
}

#[tokio::test]
async fn recover_key_metadata_looks_up_no_owner_for_an_ownerless_key_outside_the_key_set() {
    let store = PostgresStore { pool: None };
    let resolved = BTreeMap::from([(
        "sk-orphan".to_owned(),
        KeyMetadataRow {
            api_key: "sk-orphan".to_owned(),
            key_alias: Some("orphan-alias".to_owned()),
            team_id: None,
            user_id: None,
            user_email: None,
            key_exists: false,
            tags: Vec::new(),
        },
    )]);
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    let recovered = recover(&reads(&store), resolved.clone(), &keys(&[]), None).await;
    drop(guard);
    assert_eq!(recovered, resolved);
    assert!(logs.contents().is_empty());
}

async fn read_through(
    connection: &Client,
    store: &PostgresStore,
    history: &Arc<SpendLogHistory>,
    api_keys: &BTreeSet<String>,
    spent: Option<SpendLogsWindow>,
) -> BTreeMap<String, KeyMetadataRow> {
    let repository = daily_activity_repository(
        PostgresDailyActivityDatabase {
            writer: connection,
            reader: None,
        },
        store.clone(),
        Arc::clone(history),
    );
    repository.key_metadata(api_keys, spent).await.unwrap()
}

#[tokio::test]
async fn recover_key_metadata_repository_factory_attaches_user_details() {
    let database = database().await;
    insert_user(
        &database.store,
        "factory-user",
        Some("factory@example.com"),
        &[],
    )
    .await;
    insert_token(
        &database.store,
        "sk-factory",
        "factory-alias",
        Some("factory-user"),
    )
    .await;
    let connection = client(&database.store).await;
    let history = Arc::new(SpendLogHistory::default());
    let wanted = keys(&["sk-factory"]);
    let recovered = read_through(&connection, &database.store, &history, &wanted, None).await;
    assert_eq!(
        recovered.get("sk-factory"),
        Some(&KeyMetadataRow {
            key_alias: Some("factory-alias".to_owned()),
            user_email: Some("factory@example.com".to_owned()),
            key_exists: true,
            ..resolved_row("sk-factory", "factory-user")
        })
    );
    drop(connection);
    close(database).await;
}

#[tokio::test]
async fn recover_key_metadata_history_cache_is_shared_by_repositories() {
    let database = database().await;
    insert_spend_log(
        &database.store,
        DIGEST,
        moment(2026, 1, 5, 10, 0),
        "cached-alias",
        None,
    )
    .await;
    let spent = Some(window(at(2026, 1, 4, 0, 0), at(2026, 1, 7, 0, 0)));
    let connection = client(&database.store).await;
    let history = Arc::new(SpendLogHistory::default());
    let wanted = keys(&[DIGEST]);
    let first = read_through(&connection, &database.store, &history, &wanted, spent).await;
    connection
        .execute(r#"DELETE FROM "LiteLLM_SpendLogs""#, &[])
        .await
        .unwrap();
    let second = read_through(&connection, &database.store, &history, &wanted, spent).await;
    assert_eq!(first[DIGEST].key_alias.as_deref(), Some("cached-alias"));
    assert_eq!(second.get(DIGEST), first.get(DIGEST));
    drop(connection);
    close(database).await;
}

#[tokio::test]
async fn recover_key_metadata_history_cache_does_not_cross_to_a_new_history() {
    let database = database().await;
    insert_spend_log(
        &database.store,
        DIGEST,
        moment(2026, 1, 5, 10, 0),
        "cached-alias",
        None,
    )
    .await;
    let spent = Some(window(at(2026, 1, 4, 0, 0), at(2026, 1, 7, 0, 0)));
    let connection = client(&database.store).await;
    let wanted = keys(&[DIGEST]);
    let warm = Arc::new(SpendLogHistory::default());
    let first = read_through(&connection, &database.store, &warm, &wanted, spent).await;
    connection
        .execute(r#"DELETE FROM "LiteLLM_SpendLogs""#, &[])
        .await
        .unwrap();
    let cold = Arc::new(SpendLogHistory::default());
    let fresh = read_through(&connection, &database.store, &cold, &wanted, spent).await;
    assert!(first.contains_key(DIGEST));
    assert!(!fresh.contains_key(DIGEST));
    drop(connection);
    close(database).await;
}
