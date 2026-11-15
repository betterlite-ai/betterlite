#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::daily_activity_contracts::KeyMetadataRow;
use betterlite_db::postgres_store::session::PostgresStore;
use tracing::subscriber::set_default;

use super::support::{DIGEST, capture_logs, close, database, insert_user, keys, reads, recover};

#[tokio::test]
async fn user_details_fills_email_and_skips_team_for_a_non_cli_key() {
    let database = database().await;
    insert_user(
        &database.store,
        "user-1",
        Some("user@example.com"),
        &["team-a"],
    )
    .await;
    let resolved = BTreeMap::from([(
        DIGEST.to_owned(),
        KeyMetadataRow {
            api_key: DIGEST.to_owned(),
            key_alias: Some("digest-alias".to_owned()),
            team_id: None,
            user_id: Some("user-1".to_owned()),
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
            key_alias: Some("digest-alias".to_owned()),
            team_id: None,
            user_id: Some("user-1".to_owned()),
            user_email: Some("user@example.com".to_owned()),
            key_exists: false,
            tags: Vec::new(),
        })
    );
    close(database).await;
}

#[tokio::test]
async fn user_details_fills_team_only_for_the_cli_key_of_a_shared_user() {
    let database = database().await;
    insert_user(
        &database.store,
        "user-1",
        Some("user@example.com"),
        &["team-a"],
    )
    .await;
    let resolved = BTreeMap::from([
        (
            DIGEST.to_owned(),
            KeyMetadataRow {
                api_key: DIGEST.to_owned(),
                key_alias: None,
                team_id: None,
                user_id: Some("user-1".to_owned()),
                user_email: None,
                key_exists: false,
                tags: Vec::new(),
            },
        ),
        (
            "cli-session-user-1".to_owned(),
            KeyMetadataRow {
                api_key: "cli-session-user-1".to_owned(),
                key_alias: Some("cli-session-user-1".to_owned()),
                team_id: None,
                user_id: Some("user-1".to_owned()),
                user_email: None,
                key_exists: false,
                tags: Vec::new(),
            },
        ),
    ]);
    let recovered = recover(
        &reads(&database.store),
        resolved,
        &keys(&[DIGEST, "cli-session-user-1"]),
        None,
    )
    .await;
    let digest_row = &recovered[DIGEST];
    assert_eq!(digest_row.user_email.as_deref(), Some("user@example.com"));
    assert!(digest_row.team_id.is_none());
    let cli_row = &recovered["cli-session-user-1"];
    assert_eq!(cli_row.user_email.as_deref(), Some("user@example.com"));
    assert_eq!(cli_row.team_id.as_deref(), Some("team-a"));
    close(database).await;
}

#[tokio::test]
async fn user_details_keeps_an_existing_email_and_fills_the_missing_team() {
    let database = database().await;
    insert_user(
        &database.store,
        "user-1",
        Some("table@example.com"),
        &["team-a"],
    )
    .await;
    let resolved = BTreeMap::from([(
        "cli-session-user-1".to_owned(),
        KeyMetadataRow {
            api_key: "cli-session-user-1".to_owned(),
            key_alias: Some("cli-session-user-1".to_owned()),
            team_id: None,
            user_id: Some("user-1".to_owned()),
            user_email: Some("cached@example.com".to_owned()),
            key_exists: false,
            tags: Vec::new(),
        },
    )]);
    let recovered = recover(
        &reads(&database.store),
        resolved,
        &keys(&["cli-session-user-1"]),
        None,
    )
    .await;
    let row = &recovered["cli-session-user-1"];
    assert_eq!(row.user_email.as_deref(), Some("cached@example.com"));
    assert_eq!(row.team_id.as_deref(), Some("team-a"));
    close(database).await;
}

#[tokio::test]
async fn user_details_skips_the_lookup_when_an_email_is_already_present() {
    let store = PostgresStore { pool: None };
    let resolved = BTreeMap::from([(
        DIGEST.to_owned(),
        KeyMetadataRow {
            api_key: DIGEST.to_owned(),
            key_alias: None,
            team_id: None,
            user_id: Some("known".to_owned()),
            user_email: Some("cached@example.com".to_owned()),
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

#[tokio::test]
async fn user_details_skips_a_team_when_the_user_has_several() {
    let database = database().await;
    insert_user(&database.store, "user-1", None, &["team-a", "team-b"]).await;
    let resolved = BTreeMap::from([(
        "cli-session-user-1".to_owned(),
        KeyMetadataRow {
            api_key: "cli-session-user-1".to_owned(),
            key_alias: Some("cli-session-user-1".to_owned()),
            team_id: None,
            user_id: Some("user-1".to_owned()),
            user_email: None,
            key_exists: false,
            tags: Vec::new(),
        },
    )]);
    let recovered = recover(
        &reads(&database.store),
        resolved.clone(),
        &keys(&["cli-session-user-1"]),
        None,
    )
    .await;
    assert_eq!(recovered, resolved);
    close(database).await;
}

#[tokio::test]
async fn user_details_leaves_an_unknown_user_unchanged() {
    let database = database().await;
    let resolved = BTreeMap::from([(
        DIGEST.to_owned(),
        KeyMetadataRow {
            api_key: DIGEST.to_owned(),
            key_alias: None,
            team_id: None,
            user_id: Some("ghost".to_owned()),
            user_email: None,
            key_exists: false,
            tags: Vec::new(),
        },
    )]);
    let recovered = recover(
        &reads(&database.store),
        resolved.clone(),
        &keys(&[DIGEST]),
        None,
    )
    .await;
    assert_eq!(recovered, resolved);
    close(database).await;
}

#[tokio::test]
async fn user_details_skips_an_empty_user_id() {
    let database = database().await;
    insert_user(&database.store, "", Some("empty@example.com"), &["team-a"]).await;
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
    let recovered = recover(
        &reads(&database.store),
        resolved.clone(),
        &keys(&[DIGEST]),
        None,
    )
    .await;
    assert_eq!(recovered, resolved);
    close(database).await;
}
