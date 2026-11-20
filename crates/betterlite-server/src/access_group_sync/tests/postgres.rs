use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::Duration;

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::time::timeout;
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

use crate::access_group_sync::fixtures::ScriptedCache;
use crate::access_group_sync::key::postgres::PostgresKeyMembership;
use crate::access_group_sync::key::{
    KeyMembershipDatabase, KeyMembershipStatement, KeySyncError, MovedGroupRow,
    sync_key_access_groups,
};
use crate::access_group_sync::team::{
    TEAM_ADVISORY_LOCK_SQL, TeamSyncError, reconcile_team_access_groups, sync_team_access_groups,
};

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_AccessGroupTable" (
    "access_group_id" TEXT PRIMARY KEY,
    "assigned_key_ids" TEXT[],
    "assigned_team_ids" TEXT[]
);
CREATE TABLE "LiteLLM_TeamTable" (
    "team_id" TEXT PRIMARY KEY,
    "access_group_ids" TEXT[]
);
"#;
const INSERT_GROUP_SQL: &str = r#"INSERT INTO "LiteLLM_AccessGroupTable" ("access_group_id", "assigned_key_ids", "assigned_team_ids") VALUES ($1, $2, $3)"#;
const INSERT_TEAM_SQL: &str =
    r#"INSERT INTO "LiteLLM_TeamTable" ("team_id", "access_group_ids") VALUES ($1, $2)"#;
const KEY_IDS_SQL: &str =
    r#"SELECT "assigned_key_ids" FROM "LiteLLM_AccessGroupTable" WHERE "access_group_id" = $1"#;
const TEAM_IDS_SQL: &str =
    r#"SELECT "assigned_team_ids" FROM "LiteLLM_AccessGroupTable" WHERE "access_group_id" = $1"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(super) async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["access-group-sync-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA.to_owned()])
        .resolve()
        .await
        .expect("the database fixture");
    let manager = Manager::new(
        fixture
            .options
            .url
            .expose_secret()
            .parse()
            .expect("the database url"),
        NoTls,
    );
    let pool = Pool::builder(manager)
        .max_size(4)
        .build()
        .expect("the database pool");
    (fixture, pool)
}

pub(super) async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.expect("the fixture cleanup");
}

async fn execute(pool: &Pool, statement: &str, parameters: &[&(dyn ToSql + Sync)]) {
    let client = pool.get().await.expect("a fixture connection");
    client
        .execute(statement, parameters)
        .await
        .expect("the fixture statement");
}

async fn insert_group(
    pool: &Pool,
    access_group_id: &str,
    key_ids: Option<&[String]>,
    team_ids: Option<&[String]>,
) {
    let parameters: [&(dyn ToSql + Sync); 3] = [&access_group_id, &key_ids, &team_ids];
    execute(pool, INSERT_GROUP_SQL, &parameters).await;
}

async fn insert_team(pool: &Pool, team_id: &str, access_group_ids: Option<&[String]>) {
    let parameters: [&(dyn ToSql + Sync); 2] = [&team_id, &access_group_ids];
    execute(pool, INSERT_TEAM_SQL, &parameters).await;
}

pub(super) async fn seed_groups(pool: &Pool) {
    insert_group(
        pool,
        "g1",
        Some(&["old".to_owned(), "old".to_owned()]),
        None,
    )
    .await;
    insert_group(
        pool,
        "g2",
        Some(&["old".to_owned(), "tok".to_owned()]),
        Some(&["t1".to_owned(), "t1".to_owned()]),
    )
    .await;
    insert_group(pool, "g3", None, None).await;
    insert_group(pool, "g4", Some(&[]), Some(&[])).await;
    insert_group(
        pool,
        "g5",
        Some(&["old".to_owned(), "new".to_owned(), "old".to_owned()]),
        None,
    )
    .await;
    insert_group(pool, "g6", Some(&["new".to_owned()]), None).await;
    insert_group(
        pool,
        "g7",
        Some(&["old".to_owned()]),
        Some(&["t2".to_owned()]),
    )
    .await;
    insert_group(pool, "g8", None, Some(&["t3".to_owned()])).await;
}

pub(super) async fn seed_teams(pool: &Pool) {
    insert_team(
        pool,
        "t1",
        Some(&["g1".to_owned(), "g4".to_owned(), "g8".to_owned()]),
    )
    .await;
    insert_team(pool, "t2", None).await;
}

async fn single_ids(pool: &Pool, statement: &str, access_group_id: &str) -> Option<Vec<String>> {
    let client = pool.get().await.expect("a fixture connection");
    let row = client
        .query_one(statement, &[&access_group_id])
        .await
        .expect("the access group row");
    row.get(0)
}

async fn key_ids(pool: &Pool, access_group_id: &str) -> Option<Vec<String>> {
    single_ids(pool, KEY_IDS_SQL, access_group_id).await
}

async fn team_ids(pool: &Pool, access_group_id: &str) -> Option<Vec<String>> {
    single_ids(pool, TEAM_IDS_SQL, access_group_id).await
}

fn moved_names(rows: &[MovedGroupRow]) -> Vec<String> {
    let mut names: Vec<String> = rows.iter().map(|row| row.access_group_id.clone()).collect();
    names.sort();
    names
}

const fn key_membership<'db>(
    writer: &'db PostgresStore,
    reader: &'db PostgresStore,
    availability: &'db Availability,
) -> PostgresKeyMembership<'db> {
    PostgresKeyMembership {
        client: Some(PrismaClient {
            writer,
            reader,
            availability,
        }),
    }
}

pub(super) async fn key_pinned_rows(pool: &Pool) {
    let writer = PostgresStore {
        pool: Some(pool.clone()),
    };
    let reader = PostgresStore {
        pool: Some(pool.clone()),
    };
    let availability = Availability::default();
    let membership = key_membership(&writer, &reader, &availability);
    let groups = vec![
        "g1".to_owned(),
        "g2".to_owned(),
        "g3".to_owned(),
        "g4".to_owned(),
    ];
    let moved = membership
        .move_key_membership(KeyMembershipStatement::Attach, "tok", &groups)
        .await
        .unwrap();
    assert_eq!(moved_names(&moved), vec!["g1".to_owned(), "g4".to_owned()]);
    assert_eq!(key_ids(pool, "g3").await, None);
    let moved = membership
        .move_key_membership(KeyMembershipStatement::Detach, "old", &groups)
        .await
        .unwrap();
    assert_eq!(moved_names(&moved), vec!["g1".to_owned(), "g2".to_owned()]);
    assert_eq!(key_ids(pool, "g1").await, Some(vec!["tok".to_owned()]));
    let moved = membership
        .repoint_key_membership("old", "new")
        .await
        .unwrap();
    assert_eq!(moved_names(&moved), vec!["g5".to_owned(), "g7".to_owned()]);
    assert_eq!(key_ids(pool, "g5").await, Some(vec!["new".to_owned()]));
    assert_eq!(key_ids(pool, "g6").await, Some(vec!["new".to_owned()]));
}

pub(super) async fn team_pinned_rows(pool: &Pool) {
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    {
        let mut client = store.session().await.unwrap();
        let transaction = client.transaction().await.unwrap();
        let mut affected = reconcile_team_access_groups(&transaction, "t1")
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        affected.sort();
        assert_eq!(
            affected,
            vec![
                "g1".to_owned(),
                "g2".to_owned(),
                "g4".to_owned(),
                "g8".to_owned(),
            ]
        );
    }
    assert_eq!(team_ids(pool, "g1").await, Some(vec!["t1".to_owned()]));
    assert_eq!(team_ids(pool, "g2").await, Some(Vec::new()));
    assert_eq!(team_ids(pool, "g4").await, Some(vec!["t1".to_owned()]));
    assert_eq!(
        team_ids(pool, "g8").await,
        Some(vec!["t3".to_owned(), "t1".to_owned()])
    );
    let mut client = store.session().await.unwrap();
    let transaction = client.transaction().await.unwrap();
    let affected = reconcile_team_access_groups(&transaction, "t2")
        .await
        .unwrap();
    transaction.commit().await.unwrap();
    assert_eq!(affected, vec!["g7".to_owned()]);
    assert_eq!(team_ids(pool, "g7").await, Some(Vec::new()));
}

pub(super) async fn key_partial_write(pool: &Pool) {
    let writer = PostgresStore {
        pool: Some(pool.clone()),
    };
    let reader = PostgresStore {
        pool: Some(pool.clone()),
    };
    let availability = Availability::default();
    let membership = key_membership(&writer, &reader, &availability);
    let mut cache = ScriptedCache::new();
    cache.failures = Mutex::new(BTreeMap::from([("g4".to_owned(), 7)]));
    let previous = vec!["g2".to_owned()];
    let updated = vec!["g4".to_owned()];
    let error = sync_key_access_groups(&membership, &cache, "tok", Some(&previous), Some(&updated))
        .await
        .expect_err("the attach eviction fails");
    assert!(matches!(error, KeySyncError::Cache(_)));
    assert_eq!(key_ids(pool, "g4").await, Some(vec!["tok".to_owned()]));
    assert_eq!(
        key_ids(pool, "g2").await,
        Some(vec!["old".to_owned(), "tok".to_owned()])
    );
    assert_eq!(*cache.calls.lock().expect("calls"), vec!["g4".to_owned()]);
}

pub(super) async fn rolled_back(pool: &Pool) {
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let mut client = store.session().await.unwrap();
    let transaction = client.transaction().await.unwrap();
    let mut affected = reconcile_team_access_groups(&transaction, "t1")
        .await
        .unwrap();
    affected.sort();
    assert_eq!(
        affected,
        vec![
            "g1".to_owned(),
            "g2".to_owned(),
            "g4".to_owned(),
            "g8".to_owned(),
        ]
    );
    transaction.rollback().await.unwrap();
    assert_eq!(team_ids(pool, "g1").await, None);
    assert_eq!(
        team_ids(pool, "g2").await,
        Some(vec!["t1".to_owned(), "t1".to_owned()])
    );
    assert_eq!(team_ids(pool, "g8").await, Some(vec!["t3".to_owned()]));
}

pub(super) async fn commit_then_cache_failure(pool: &Pool) {
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let mut cache = ScriptedCache::new();
    cache.failures = Mutex::new(BTreeMap::from([("g1".to_owned(), 3)]));
    let error = sync_team_access_groups(&store, &cache, "t1")
        .await
        .expect_err("the eviction fails after the commit");
    assert!(matches!(error, TeamSyncError::Cache(_)));
    assert_eq!(team_ids(pool, "g1").await, Some(vec!["t1".to_owned()]));
    assert_eq!(team_ids(pool, "g2").await, Some(Vec::new()));
    assert_eq!(team_ids(pool, "g4").await, Some(vec!["t1".to_owned()]));
}

pub(super) async fn key_degraded_writer_writes(pool: &Pool) {
    let writer = PostgresStore {
        pool: Some(pool.clone()),
    };
    let reader = PostgresStore {
        pool: Some(pool.clone()),
    };
    let availability = Availability::default();
    availability
        .writer_unavailable
        .store(true, Ordering::SeqCst);
    let membership = key_membership(&writer, &reader, &availability);
    let groups = vec!["g1".to_owned()];
    let moved = membership
        .move_key_membership(KeyMembershipStatement::Attach, "tok", &groups)
        .await
        .unwrap();
    assert_eq!(moved_names(&moved), vec!["g1".to_owned()]);
    assert_eq!(
        key_ids(pool, "g1").await,
        Some(vec!["old".to_owned(), "old".to_owned(), "tok".to_owned()])
    );
}

pub(super) async fn concurrent_team_appends(pool: &Pool) {
    insert_group(pool, "gc", Some(&[]), Some(&[])).await;
    insert_team(pool, "ta", Some(&["gc".to_owned()])).await;
    insert_team(pool, "tb", Some(&["gc".to_owned()])).await;
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let cache = ScriptedCache::new();
    let left_sync = sync_team_access_groups(&store, &cache, "ta");
    let right_sync = sync_team_access_groups(&store, &cache, "tb");
    let (left, right) = tokio::join!(left_sync, right_sync);
    left.unwrap();
    right.unwrap();
    let mut teams = team_ids(pool, "gc").await.unwrap();
    teams.sort();
    assert_eq!(teams, vec!["ta".to_owned(), "tb".to_owned()]);
}

pub(super) async fn team_lock_blocks_until_released(pool: &Pool) {
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let mut holder = store.session().await.unwrap();
    let holding = holder.transaction().await.unwrap();
    holding
        .query(TEAM_ADVISORY_LOCK_SQL, &[&"t1"])
        .await
        .expect("the held team lock");
    let cache = ScriptedCache::new();
    let mut pending = Box::pin(sync_team_access_groups(&store, &cache, "t1"));
    let waiting = timeout(Duration::from_secs(1), &mut pending).await;
    assert!(
        waiting.is_err(),
        "the sync waits while another transaction holds the team lock"
    );
    assert_eq!(team_ids(pool, "g1").await, None);
    holding.rollback().await.unwrap();
    timeout(Duration::from_secs(5), pending)
        .await
        .expect("the sync finishes once the lock is released")
        .unwrap();
    assert_eq!(team_ids(pool, "g1").await, Some(vec!["t1".to_owned()]));
}

pub(super) async fn retry_evicts_desired(pool: &Pool) {
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let mut failing = ScriptedCache::new();
    failing.failures = Mutex::new(BTreeMap::from([("g2".to_owned(), 5)]));
    sync_team_access_groups(&store, &failing, "t1")
        .await
        .expect_err("the first eviction fails");
    let retry = ScriptedCache::new();
    sync_team_access_groups(&store, &retry, "t1").await.unwrap();
    let mut evicted = retry.calls.lock().expect("calls").clone();
    evicted.sort();
    assert_eq!(
        evicted,
        vec!["g1".to_owned(), "g4".to_owned(), "g8".to_owned()]
    );
    assert_eq!(team_ids(pool, "g1").await, Some(vec!["t1".to_owned()]));
    assert_eq!(team_ids(pool, "g2").await, Some(Vec::new()));
}
