use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use betterlite_postgres_fixture::connection::PostgresFixture;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::ExposeSecret as _;
use tokio::time::sleep;
use tokio_postgres::NoTls;

use crate::key_attribution::history::recovery::SpendLogHistory;
use crate::key_attribution::history::window::HistoryWindow;
use crate::key_attribution::metadata::KeyMetadata;
use crate::postgres_store::session::PostgresStore;

use super::support::{DIGEST, OTHER_DIGEST, at, window};

const BASELINE: &str = include_str!("../../../../migrations/20250326162113_baseline/migration.sql");
const LOCK_SPEND_LOGS: &str = r#"BEGIN; LOCK TABLE "LiteLLM_SpendLogs" IN ACCESS EXCLUSIVE MODE"#;

#[derive(clap::Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct Database {
    fixture: PostgresFixture,
    store: PostgresStore,
    holder: Pool,
}

fn pool(fixture: &PostgresFixture, application_name: &str) -> Pool {
    let mut config: tokio_postgres::Config = fixture.options.url.expose_secret().parse().unwrap();
    config.application_name(application_name);
    Pool::builder(Manager::new(config, NoTls))
        .max_size(3)
        .build()
        .unwrap()
}

async fn database(application_name: &str) -> Database {
    use clap::Parser as _;
    let url = Environment::parse_from(["history-tests"])
        .url
        .map(secrecy::SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let store = PostgresStore {
        pool: Some(pool(&fixture, application_name)),
    };
    let holder = pool(&fixture, "history-holder");
    store
        .session()
        .await
        .unwrap()
        .batch_execute(BASELINE)
        .await
        .unwrap();
    Database {
        fixture,
        store,
        holder,
    }
}

async fn close(database: Database) {
    if let Some(store_pool) = database.store.pool {
        store_pool.close();
    }
    database.holder.close();
    database.fixture.close_async().await.unwrap();
}

fn alias_json(alias: &str) -> String {
    format!(r#"{{"user_api_key_alias":"{alias}"}}"#)
}

fn meta(alias: Option<&str>, team: Option<&str>, user: Option<&str>) -> KeyMetadata {
    KeyMetadata {
        key_alias: alias.map(str::to_owned),
        team_id: team.map(str::to_owned),
        user_id: user.map(str::to_owned),
        user_email: None,
        key_exists: None,
    }
}

async fn insert_spend(
    store: &PostgresStore,
    api_key: &str,
    second: i64,
    columns: (Option<&str>, Option<&str>),
    metadata: &str,
) {
    let request_id = uuid::Uuid::new_v4().to_string();
    store
        .session()
        .await
        .unwrap()
        .execute(
            r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, "startTime", "endTime", team_id, "user", metadata) VALUES ($1, 'completion', $2, $3, $3, $4, $5, $6::text::jsonb)"#,
            &[&request_id, &api_key, &at(second), &columns.0, &columns.1, &metadata],
        )
        .await
        .unwrap();
}

async fn insert_unnamed_series(store: &PostgresStore, api_key: &str, first: i64, count: i64) {
    store
        .session()
        .await
        .unwrap()
        .execute(
            r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, "startTime", "endTime")
SELECT gen_random_uuid()::text, 'completion', $1::text, $2::timestamp + n * interval '1 second', $2::timestamp + n * interval '1 second'
FROM generate_series(0, $3::bigint - 1) AS n"#,
            &[&api_key, &at(first), &count],
        )
        .await
        .unwrap();
}

async fn recover(
    history: &SpendLogHistory,
    store: &PostgresStore,
    keys: &[&str],
    probed: &HistoryWindow,
) -> BTreeMap<String, KeyMetadata> {
    let requested: BTreeSet<String> = keys.iter().map(|key| (*key).to_owned()).collect();
    history
        .recover_key_metadata(store, &requested, probed)
        .await
        .unwrap()
}

async fn waiting_queries(monitor: &Client, application_name: &str, attempts: u32) -> i64 {
    let mut waiting = 0_i64;
    for _ in 0..attempts {
        waiting = monitor
            .query_one(
                "SELECT count(*) FROM pg_stat_activity WHERE application_name = $1 AND wait_event_type = 'Lock'",
                &[&application_name],
            )
            .await
            .unwrap()
            .get(0);
        if waiting > 0 {
            break;
        }
        sleep(Duration::from_millis(25)).await;
    }
    waiting
}

async fn insert_one_and_two(store: &PostgresStore) {
    insert_spend(store, DIGEST, 10, (None, None), &alias_json("one")).await;
    insert_spend(store, OTHER_DIGEST, 10, (None, None), &alias_json("two")).await;
}

#[tokio::test]
async fn postgres_first_and_last_disagreement_is_resolved_per_field() {
    let database = database("history-fields").await;
    let store = &database.store;
    let columns = (Some("team-a"), Some("user-a"));
    insert_spend(store, DIGEST, 10, columns, &alias_json("old-alias")).await;
    insert_spend(store, DIGEST, 20, (None, None), "{}").await;
    insert_spend(
        store,
        DIGEST,
        30,
        (Some("team-a"), None),
        &alias_json("new-alias"),
    )
    .await;
    insert_spend(
        store,
        OTHER_DIGEST,
        10,
        (Some("t1"), Some("u1")),
        &alias_json("a"),
    )
    .await;
    insert_spend(
        store,
        OTHER_DIGEST,
        20,
        (Some("t2"), Some("u2")),
        &alias_json("b"),
    )
    .await;
    let history = SpendLogHistory::default();
    let recovered = recover(&history, store, &[DIGEST, OTHER_DIGEST], &window(0, 100)).await;
    assert_eq!(
        recovered,
        BTreeMap::from([(
            DIGEST.to_owned(),
            meta(None, Some("team-a"), Some("user-a"))
        )])
    );
    close(database).await;
}

#[tokio::test]
async fn postgres_alias_comes_from_metadata_and_team_and_user_prefer_row_columns() {
    let database = database("history-columns").await;
    let store = &database.store;
    let both = r#"{"user_api_key_alias":"alias","user_api_key_team_id":"meta-team","user_api_key_user_id":"meta-user"}"#;
    insert_spend(
        store,
        DIGEST,
        10,
        (Some("col-team"), Some("col-user")),
        both,
    )
    .await;
    let fallback = r#"{"user_api_key_alias":"","user_api_key_team_id":"meta-team","user_api_key_user_id":"meta-user"}"#;
    insert_spend(store, OTHER_DIGEST, 10, (Some(""), Some("")), fallback).await;
    let history = SpendLogHistory::default();
    let recovered = recover(&history, store, &[DIGEST, OTHER_DIGEST], &window(0, 100)).await;
    assert_eq!(
        recovered,
        BTreeMap::from([
            (
                DIGEST.to_owned(),
                meta(Some("alias"), Some("col-team"), Some("col-user"))
            ),
            (
                OTHER_DIGEST.to_owned(),
                meta(None, Some("meta-team"), Some("meta-user"))
            ),
        ])
    );
    close(database).await;
}

#[tokio::test]
async fn postgres_a_row_with_only_a_user_column_names_the_user_not_an_alias() {
    let database = database("history-user-only").await;
    insert_spend(&database.store, DIGEST, 10, (None, Some("user-a")), "{}").await;
    let history = SpendLogHistory::default();
    let recovered = recover(&history, &database.store, &[DIGEST], &window(0, 100)).await;
    assert_eq!(
        recovered,
        BTreeMap::from([(DIGEST.to_owned(), meta(None, None, Some("user-a")))])
    );
    close(database).await;
}

#[tokio::test]
async fn postgres_an_unnamed_oldest_window_still_probes_the_newest_rows() {
    let database = database("history-unnamed-oldest").await;
    let store = &database.store;
    insert_unnamed_series(store, DIGEST, 0, 100).await;
    insert_spend(store, DIGEST, 1000, (None, None), &alias_json("late")).await;
    let history = SpendLogHistory::default();
    let recovered = recover(&history, store, &[DIGEST], &window(0, 5000)).await;
    assert_eq!(
        recovered,
        BTreeMap::from([(DIGEST.to_owned(), meta(Some("late"), None, None))])
    );
    close(database).await;
}

#[tokio::test]
async fn postgres_a_named_row_between_the_two_probes_is_not_searched_for() {
    let database = database("history-between").await;
    let store = &database.store;
    insert_unnamed_series(store, DIGEST, 0, 100).await;
    insert_spend(store, DIGEST, 100, (None, None), &alias_json("hidden")).await;
    insert_unnamed_series(store, DIGEST, 101, 150).await;
    let history = SpendLogHistory::default();
    let recovered = recover(&history, store, &[DIGEST], &window(0, 5000)).await;
    assert!(recovered.is_empty());
    close(database).await;
}

#[tokio::test]
async fn postgres_the_oldest_named_row_inside_the_first_hundred_is_found() {
    let database = database("history-first-hundred").await;
    let store = &database.store;
    insert_unnamed_series(store, DIGEST, 0, 99).await;
    insert_spend(store, DIGEST, 99, (None, None), &alias_json("hundredth")).await;
    insert_unnamed_series(store, DIGEST, 100, 300).await;
    let history = SpendLogHistory::default();
    let recovered = recover(&history, store, &[DIGEST], &window(0, 5000)).await;
    assert_eq!(
        recovered,
        BTreeMap::from([(DIGEST.to_owned(), meta(Some("hundredth"), None, None))])
    );
    close(database).await;
}

#[tokio::test]
async fn postgres_equal_timestamps_still_find_the_named_row() {
    let database = database("history-ties").await;
    let store = &database.store;
    insert_spend(store, DIGEST, 10, (None, None), "{}").await;
    insert_spend(store, DIGEST, 10, (None, None), &alias_json("tied")).await;
    insert_spend(store, DIGEST, 10, (None, None), "{}").await;
    let history = SpendLogHistory::default();
    let recovered = recover(&history, store, &[DIGEST], &window(0, 100)).await;
    assert_eq!(
        recovered,
        BTreeMap::from([(DIGEST.to_owned(), meta(Some("tied"), None, None))])
    );
    close(database).await;
}

#[tokio::test]
async fn postgres_the_window_includes_its_start_and_excludes_its_end() {
    let database = database("history-half-open").await;
    let store = &database.store;
    insert_spend(store, DIGEST, 100, (None, None), &alias_json("at-start")).await;
    insert_spend(
        store,
        OTHER_DIGEST,
        200,
        (None, None),
        &alias_json("at-end"),
    )
    .await;
    let history = SpendLogHistory::default();
    let recovered = recover(&history, store, &[DIGEST, OTHER_DIGEST], &window(100, 200)).await;
    assert_eq!(
        recovered,
        BTreeMap::from([(DIGEST.to_owned(), meta(Some("at-start"), None, None))])
    );
    close(database).await;
}

#[tokio::test]
async fn postgres_a_jwt_prefixed_key_is_queried_and_returned_under_its_original_name() {
    let database = database("history-prefix").await;
    let store = &database.store;
    let prefixed = format!("hashed-jwt-{DIGEST}");
    let doubled = format!("hashed-jwt-{prefixed}");
    insert_spend(store, &prefixed, 10, (None, None), &alias_json("jwt-alias")).await;
    insert_spend(store, DIGEST, 10, (None, None), &alias_json("plain-alias")).await;
    insert_spend(
        store,
        &doubled,
        10,
        (None, None),
        &alias_json("doubled-alias"),
    )
    .await;
    let history = SpendLogHistory::default();
    let keys = [prefixed.as_str(), doubled.as_str(), "sk-not-a-digest"];
    let recovered = recover(&history, store, &keys, &window(0, 100)).await;
    assert_eq!(
        recovered,
        BTreeMap::from([(prefixed.clone(), meta(Some("jwt-alias"), None, None))])
    );
    close(database).await;
}

#[tokio::test]
async fn postgres_a_success_is_served_from_the_cache_for_the_same_window_only() {
    let database = database("history-window-cache").await;
    insert_spend(
        &database.store,
        DIGEST,
        10,
        (None, None),
        &alias_json("cached"),
    )
    .await;
    let history = SpendLogHistory::default();
    let probed = window(0, 100);
    let found = BTreeMap::from([(DIGEST.to_owned(), meta(Some("cached"), None, None))]);
    assert_eq!(
        recover(&history, &database.store, &[DIGEST], &probed).await,
        found
    );
    let offline = PostgresStore { pool: None };
    assert_eq!(recover(&history, &offline, &[DIGEST], &probed).await, found);
    let shifted = window(0, 101);
    assert!(
        recover(&history, &offline, &[DIGEST], &shifted)
            .await
            .is_empty()
    );
    let unseen = window(0, 102);
    assert_eq!(
        recover(&history, &database.store, &[DIGEST], &unseen).await,
        found
    );
    close(database).await;
}

#[tokio::test]
async fn postgres_a_miss_is_remembered_even_when_rows_arrive_later() {
    let database = database("history-miss-cache").await;
    let history = SpendLogHistory::default();
    let probed = window(0, 100);
    assert!(
        recover(&history, &database.store, &[DIGEST], &probed)
            .await
            .is_empty()
    );
    insert_spend(
        &database.store,
        DIGEST,
        10,
        (None, None),
        &alias_json("late"),
    )
    .await;
    assert!(
        recover(&history, &database.store, &[DIGEST], &probed)
            .await
            .is_empty()
    );
    let other_window = window(0, 101);
    assert_eq!(
        recover(&history, &database.store, &[DIGEST], &other_window).await,
        BTreeMap::from([(DIGEST.to_owned(), meta(Some("late"), None, None))])
    );
    close(database).await;
}

#[tokio::test]
async fn postgres_a_missing_database_counts_as_a_miss_without_failing() {
    let offline = PostgresStore { pool: None };
    let history = SpendLogHistory::default();
    assert!(
        recover(&history, &offline, &[DIGEST], &window(0, 100))
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn postgres_two_concurrent_callers_share_one_query() {
    let database = database("history-concurrent").await;
    insert_spend(
        &database.store,
        DIGEST,
        10,
        (None, None),
        &alias_json("shared"),
    )
    .await;
    let monitor = database.holder.get().await.unwrap();
    monitor.batch_execute(LOCK_SPEND_LOGS).await.unwrap();
    let history = SpendLogHistory::default();
    let probed = window(0, 100);
    let callers = async {
        tokio::join!(
            recover(&history, &database.store, &[DIGEST], &probed),
            recover(&history, &database.store, &[DIGEST], &probed)
        )
    };
    let watcher = async {
        let waiting = waiting_queries(&monitor, "history-concurrent", 100).await;
        assert_eq!(waiting, 1);
        sleep(Duration::from_millis(400)).await;
        assert_eq!(waiting_queries(&monitor, "history-concurrent", 1).await, 1);
        monitor.batch_execute("COMMIT").await.unwrap();
    };
    let ((from_first, from_second), ()) = tokio::join!(callers, watcher);
    let found = BTreeMap::from([(DIGEST.to_owned(), meta(Some("shared"), None, None))]);
    assert_eq!(from_first, found);
    assert_eq!(from_second, found);
    drop(monitor);
    close(database).await;
}

#[tokio::test]
async fn postgres_a_timed_out_query_is_a_miss_and_a_waiting_caller_queries_only_what_is_missing() {
    let database = database("history-timeout").await;
    insert_one_and_two(&database.store).await;
    let holder = database.holder.get().await.unwrap();
    holder.batch_execute(LOCK_SPEND_LOGS).await.unwrap();
    let history = SpendLogHistory::default();
    let probed = window(0, 100);
    let started = Instant::now();
    let first_caller = async {
        let recovered = recover(&history, &database.store, &[DIGEST], &probed).await;
        holder.batch_execute("COMMIT").await.unwrap();
        recovered
    };
    let second_caller = recover(&history, &database.store, &[DIGEST, OTHER_DIGEST], &probed);
    let (from_first, from_second) = tokio::join!(first_caller, second_caller);
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_millis(4900), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(9), "{elapsed:?}");
    assert!(from_first.is_empty());
    assert_eq!(
        from_second,
        BTreeMap::from([(OTHER_DIGEST.to_owned(), meta(Some("two"), None, None))])
    );
    assert!(
        recover(&history, &database.store, &[DIGEST], &probed)
            .await
            .is_empty()
    );
    drop(holder);
    close(database).await;
}
