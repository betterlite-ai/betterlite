use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};

use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_db::postgres_store::session::PostgresStore;

use crate::team_list_query::TeamListQueryError;
use crate::team_list_query::lookup::{org_admin_org_ids, user_team_ids_from_db};

use super::support::{RecordingCache, database, lookup, user, user_insert, user_store};

#[tokio::test]
async fn org_lookup_value_error() {
    let fixture = database(&user_insert("u1", &["t1"])).await;
    let cache = Arc::new(RecordingCache::default());
    let store = user_store(&fixture.store, &fixture.store);
    let missing = org_admin_org_ids(&lookup(&cache, Some(store)), "missing")
        .await
        .unwrap();
    assert_eq!(missing, None);
    // _user_read_failure turns an ordinary read failure into the same ValueError
    let broken = PostgresStore { pool: None };
    let failing = lookup(&cache, Some(user_store(&broken, &broken)));
    let unreadable = org_admin_org_ids(&failing, "u1").await.unwrap();
    assert_eq!(unreadable, None);
    fixture.close().await;
}

#[tokio::test]
async fn direct_teams_bypass_cache() {
    let fixture = database(&user_insert("u1", &["db-team"])).await;
    let cache = Arc::new(RecordingCache {
        user: Some(user(r#"{"user_id":"u1","teams":["cached-team"]}"#)),
        reads: Mutex::new(Vec::new()),
        writes: AtomicUsize::new(0),
    });
    // check_db_only reads use_writer=True, so a dead reader must not be touched
    let broken = PostgresStore { pool: None };
    let store = user_store(&broken, &fixture.store);
    let teams = user_team_ids_from_db(&lookup(&cache, Some(store)), "u1")
        .await
        .unwrap();
    assert_eq!(teams, vec!["db-team".to_owned()]);
    assert!(cache.read_keys().is_empty());
    fixture.close().await;
}

#[tokio::test]
async fn unexpected_lookup_error_propagates() {
    let cache = Arc::new(RecordingCache::default());
    let no_database = lookup(&cache, None);
    let org_error = org_admin_org_ids(&no_database, "u1").await.unwrap_err();
    assert!(matches!(
        org_error,
        TeamListQueryError::Lookup(UserLookupError::NoDatabase)
    ));
    let teams_error = user_team_ids_from_db(&no_database, "u1").await.unwrap_err();
    assert!(matches!(
        teams_error,
        TeamListQueryError::Lookup(UserLookupError::NoDatabase)
    ));
}
