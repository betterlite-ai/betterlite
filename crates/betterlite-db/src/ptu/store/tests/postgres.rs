use std::collections::BTreeSet;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use deadpool_postgres::Client;
use rust_decimal::Decimal;
use tokio_postgres::types::Json;

use super::support::{Recorder, capture_logs, charge, database, write_for};
use crate::budget_permission_rows::JsonData;
use crate::ptu::charges::deployment::RouterEntry;
use crate::ptu::store::StoreError;
use crate::ptu::store::models::load_models;
use crate::ptu::store::prune::prune_unrefreshed_rows;
use crate::ptu::store::sentinel::{SentinelKey, existing_sentinel_keys, write_charge_with_retry};

const DAY: &str = "2026-03-01";
const SENTINEL: &str = "__ptu_flat_cost__";

fn day(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
}

fn instant(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

fn info(source: &str) -> JsonData {
    serde_json::from_str(source).unwrap()
}

async fn seed(client: &Client, rows: &str) {
    let sql = format!(
        "INSERT INTO \"LiteLLM_DailyTeamSpend\" \
         (\"id\", \"team_id\", \"date\", \"api_key\", \"model\", \"model_group\", \
          \"custom_llm_provider\", \"mcp_namespaced_tool_name\", \"endpoint\", \"spend\", \
          \"api_requests\", \"prompt_tokens\", \"created_at\", \"updated_at\", \"ptu_flat_cost\") \
         VALUES {rows}"
    );
    client.batch_execute(&sql).await.unwrap();
}

async fn row_count(client: &Client) -> i64 {
    client
        .query_one("SELECT COUNT(*) FROM \"LiteLLM_DailyTeamSpend\"", &[])
        .await
        .unwrap()
        .get(0)
}

async fn fail_upserts_for(client: &Client, model: &str, limit: i32) {
    let sql = format!(
        "CREATE SEQUENCE ptu_upsert_attempts; \
         CREATE FUNCTION ptu_flaky_upsert() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN \
           IF NEW.model = '{model}' AND nextval('ptu_upsert_attempts') <= {limit} THEN \
             RAISE EXCEPTION 'transient upsert failure'; \
           END IF; \
           RETURN NEW; \
         END; \
         $$; \
         CREATE TRIGGER ptu_flaky_upsert_trigger BEFORE INSERT OR UPDATE \
         ON \"LiteLLM_DailyTeamSpend\" FOR EACH ROW EXECUTE FUNCTION ptu_flaky_upsert();"
    );
    client.batch_execute(&sql).await.unwrap();
}

async fn upsert_attempts(client: &Client) -> i64 {
    client
        .query_one("SELECT nextval('ptu_upsert_attempts')", &[])
        .await
        .unwrap()
        .get(0)
}

#[tokio::test]
async fn an_update_replaces_the_charge_and_the_display_name() {
    let (_fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    seed(
        &client,
        "('seed', 'team-1', '2026-03-01', '__ptu_flat_cost__', 'model-a', 'old', \
          '', '', '', 9.5, 2, 7, '2000-01-01 00:00:00', '2000-01-01 00:00:00', 3.5)",
    )
    .await;
    let charge = charge("team-1", "model-a", "new", Decimal::new(125, 2));
    let write = write_for(&charge, DAY);
    let mut sleeper = Recorder::default();
    assert!(write_charge_with_retry(&client, &mut sleeper, &write).await);
    assert!(sleeper.delays.is_empty());
    let row = client
        .query_one(
            "SELECT \"model_group\", \"ptu_flat_cost\"::text, \"updated_at\"::text, \
             \"created_at\"::text, \"spend\"::text, \"api_requests\", \"prompt_tokens\" \
             FROM \"LiteLLM_DailyTeamSpend\" WHERE \"id\" = 'seed'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>("model_group"), "new");
    assert_eq!(row.get::<_, String>("ptu_flat_cost"), "1.25");
    assert_eq!(row.get::<_, String>("updated_at"), "2026-03-01 12:00:00");
    assert_eq!(row.get::<_, String>("created_at"), "2000-01-01 00:00:00");
    assert_eq!(row.get::<_, String>("spend"), "9.5");
    assert_eq!(row.get::<_, i64>("api_requests"), 2);
    assert_eq!(row.get::<_, i64>("prompt_tokens"), 7);
    assert_eq!(row_count(&client).await, 1);
}

#[tokio::test]
async fn a_create_writes_the_identity_columns_and_leaves_defaults() {
    let (_fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    let charge = charge("team-1", "model-a", "display", Decimal::from(3));
    let write = write_for(&charge, DAY);
    let id = write.id.to_string();
    let mut sleeper = Recorder::default();
    assert!(write_charge_with_retry(&client, &mut sleeper, &write).await);
    let row = client
        .query_one(
            "SELECT \"api_key\", \"model\", \"model_group\", \"custom_llm_provider\", \
             \"mcp_namespaced_tool_name\", \"endpoint\", \"date\", \"ptu_flat_cost\"::text, \
             \"updated_at\"::text, \"created_at\" IS NOT NULL AS created, \"spend\"::text, \
             \"api_requests\", \"prompt_tokens\", \"completion_tokens\", \"successful_requests\" \
             FROM \"LiteLLM_DailyTeamSpend\" WHERE \"id\" = $1",
            &[&id],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>("api_key"), SENTINEL);
    assert_eq!(row.get::<_, String>("model"), "model-a");
    assert_eq!(row.get::<_, String>("model_group"), "display");
    assert_eq!(row.get::<_, String>("custom_llm_provider"), "");
    assert_eq!(row.get::<_, String>("mcp_namespaced_tool_name"), "");
    assert_eq!(row.get::<_, String>("endpoint"), "");
    assert_eq!(row.get::<_, String>("date"), DAY);
    assert_eq!(row.get::<_, String>("ptu_flat_cost"), "3");
    assert_eq!(row.get::<_, String>("updated_at"), "2026-03-01 12:00:00");
    assert!(row.get::<_, bool>("created"));
    assert_eq!(row.get::<_, String>("spend"), "0");
    assert_eq!(row.get::<_, i64>("api_requests"), 0);
    assert_eq!(row.get::<_, i64>("prompt_tokens"), 0);
    assert_eq!(row.get::<_, i64>("completion_tokens"), 0);
    assert_eq!(row.get::<_, i64>("successful_requests"), 0);
}

#[tokio::test]
async fn a_permanent_failure_retries_three_times_and_reports_false() {
    let (_fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    fail_upserts_for(&client, "model-a", 100).await;
    let charge = charge("team-1", "model-a", "display", Decimal::ONE);
    let write = write_for(&charge, DAY);
    let mut sleeper = Recorder::default();
    let (logs, subscriber) = capture_logs();
    let guard = tracing::subscriber::set_default(subscriber);
    let written = write_charge_with_retry(&client, &mut sleeper, &write).await;
    drop(guard);
    assert!(!written);
    assert_eq!(
        sleeper.delays,
        [Duration::from_millis(500), Duration::from_millis(1000)]
    );
    assert_eq!(upsert_attempts(&client).await, 4);
    assert_eq!(row_count(&client).await, 0);
    let captured = logs.contents();
    assert!(
        captured.contains("rerun the rollup for that date to recover"),
        "{captured}"
    );
    assert!(captured.contains("model=display"), "{captured}");
}

#[tokio::test]
async fn transient_failures_recover_on_the_third_attempt() {
    let (_fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    fail_upserts_for(&client, "model-a", 2).await;
    let charge = charge("team-1", "model-a", "display", Decimal::ONE);
    let write = write_for(&charge, DAY);
    let mut sleeper = Recorder::default();
    assert!(write_charge_with_retry(&client, &mut sleeper, &write).await);
    assert_eq!(
        sleeper.delays,
        [Duration::from_millis(500), Duration::from_millis(1000)]
    );
    assert_eq!(upsert_attempts(&client).await, 4);
    assert_eq!(row_count(&client).await, 1);
}

#[tokio::test]
async fn a_failed_charge_leaves_another_charges_row_in_place() {
    let (_fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    fail_upserts_for(&client, "model-a", 100).await;
    let mut sleeper = Recorder::default();
    let failing = charge("team-1", "model-a", "first", Decimal::ONE);
    let failing_write = write_for(&failing, DAY);
    assert!(!write_charge_with_retry(&client, &mut sleeper, &failing_write).await);
    let other = charge("team-1", "model-b", "second", Decimal::ONE);
    let other_write = write_for(&other, DAY);
    assert!(write_charge_with_retry(&client, &mut sleeper, &other_write).await);
    assert_eq!(row_count(&client).await, 1);
    let row = client
        .query_one("SELECT \"model\" FROM \"LiteLLM_DailyTeamSpend\"", &[])
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>("model"), "model-b");
}

#[tokio::test]
async fn existing_keys_read_sentinel_rows_in_the_inclusive_range() {
    let (_fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    seed(
        &client,
        "('k-start', 'team-1', '2026-02-28', '__ptu_flat_cost__', 'model-a', 'name-a', '', '', '', 0, 0, 0, now(), '2026-02-28 00:00:00', 0), \
         ('k-end', 'team-1', '2026-03-02', '__ptu_flat_cost__', 'model-b', 'name-b', '', '', '', 0, 0, 0, now(), '2026-03-02 00:00:00', 0), \
         ('k-out', 'team-1', '2026-03-03', '__ptu_flat_cost__', 'model-c', 'name-c', '', '', '', 0, 0, 0, now(), '2026-03-03 00:00:00', 0), \
         ('k-usage', 'team-1', '2026-03-01', 'key-1', 'model-d', 'name-d', '', '', '', 1.5, 1, 0, now(), '2026-03-01 00:00:00', 0), \
         ('k-group', 'team-2', '2026-03-01', '__ptu_flat_cost__', 'model-e', 'other-name', '', '', '', 0, 0, 0, now(), '2026-03-01 00:00:00', 0)",
    )
    .await;
    let keys = existing_sentinel_keys(&client, day("2026-02-28"), day("2026-03-02"))
        .await
        .unwrap();
    assert_eq!(
        keys,
        BTreeSet::from([
            SentinelKey {
                team_id: "team-1".to_owned(),
                model: "model-a".to_owned(),
                date: "2026-02-28".to_owned(),
            },
            SentinelKey {
                team_id: "team-1".to_owned(),
                model: "model-b".to_owned(),
                date: "2026-03-02".to_owned(),
            },
            SentinelKey {
                team_id: "team-2".to_owned(),
                model: "model-e".to_owned(),
                date: "2026-03-01".to_owned(),
            },
        ])
    );
}

#[tokio::test]
async fn null_identity_columns_read_as_empty_text() {
    let (_fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    seed(
        &client,
        "('k-null', NULL, '2026-03-01', '__ptu_flat_cost__', NULL, 'name', '', '', '', 0, 0, 0, now(), '2026-03-01 00:00:00', 0)",
    )
    .await;
    let keys = existing_sentinel_keys(&client, day(DAY), day(DAY))
        .await
        .unwrap();
    assert_eq!(
        keys,
        BTreeSet::from([SentinelKey {
            team_id: String::new(),
            model: String::new(),
            date: DAY.to_owned(),
        }])
    );
}

#[tokio::test]
async fn prune_deletes_only_stale_scanned_sentinel_rows_of_the_day() {
    let (_fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    seed(
        &client,
        "('stale', 'team-1', '2026-03-01', '__ptu_flat_cost__', 'model-a', 'n', '', '', '', 0, 0, 0, now(), '2026-03-01 11:54:59', 0), \
         ('edge', 'team-1', '2026-03-01', '__ptu_flat_cost__', 'model-b', 'n', '', '', '', 0, 0, 0, now(), '2026-03-01 11:55:00', 0), \
         ('fresh', 'team-1', '2026-03-01', '__ptu_flat_cost__', 'model-c', 'n', '', '', '', 0, 0, 0, now(), '2026-03-01 11:55:01', 0), \
         ('unscanned', 'team-1', '2026-03-01', '__ptu_flat_cost__', 'model-d', 'n', '', '', '', 0, 0, 0, now(), '2026-03-01 10:00:00', 0), \
         ('other-day', 'team-1', '2026-02-28', '__ptu_flat_cost__', 'model-e', 'n', '', '', '', 0, 0, 0, now(), '2026-02-28 10:00:00', 0), \
         ('usage', 'team-1', '2026-03-01', 'key-1', 'model-f', 'n', '', '', '', 0, 0, 0, now(), '2026-03-01 10:00:00', 0), \
         ('other-team', 'team-2', '2026-03-01', '__ptu_flat_cost__', 'model-g', 'n', '', '', '', 0, 0, 0, now(), '2026-03-01 10:00:00', 0)",
    )
    .await;
    let scanned: BTreeSet<String> = [
        "model-a", "model-b", "model-c", "model-e", "model-f", "model-g",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let (logs, subscriber) = capture_logs();
    let guard = tracing::subscriber::set_default(subscriber);
    let deleted =
        prune_unrefreshed_rows(&client, day(DAY), instant("2026-03-01T12:00:00Z"), &scanned)
            .await
            .unwrap();
    drop(guard);
    assert_eq!(deleted, 2);
    assert!(
        logs.contents()
            .contains("pruned 2 stale sentinel row(s) of 6 deployment(s) considered")
    );
    let rows = client
        .query(
            "SELECT \"id\" FROM \"LiteLLM_DailyTeamSpend\" ORDER BY \"id\"",
            &[],
        )
        .await
        .unwrap();
    let ids: Vec<String> = rows.iter().map(|row| row.get("id")).collect();
    assert_eq!(ids, ["edge", "fresh", "other-day", "unscanned", "usage"]);
}

#[tokio::test]
async fn an_empty_scan_deletes_nothing() {
    let (_fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    seed(
        &client,
        "('stale', 'team-1', '2026-03-01', '__ptu_flat_cost__', 'model-a', 'n', '', '', '', 0, 0, 0, now(), '2026-03-01 11:00:00', 0)",
    )
    .await;
    let (logs, subscriber) = capture_logs();
    let guard = tracing::subscriber::set_default(subscriber);
    let deleted = prune_unrefreshed_rows(
        &client,
        day(DAY),
        instant("2026-03-01T12:00:00Z"),
        &BTreeSet::new(),
    )
    .await
    .unwrap();
    drop(guard);
    assert_eq!(deleted, 0);
    assert!(!logs.contents().contains("pruned"));
    assert_eq!(row_count(&client).await, 1);
}

#[tokio::test]
async fn a_cutoff_before_the_first_instant_is_an_error() {
    let (_fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    let result = prune_unrefreshed_rows(
        &client,
        day(DAY),
        DateTime::<Utc>::MIN_UTC,
        &BTreeSet::new(),
    )
    .await;
    assert!(matches!(result, Err(StoreError::GraceUnderflow { .. })));
}

fn router_entry(source: &str) -> RouterEntry {
    RouterEntry {
        model_name: "router-name".to_owned(),
        model_info: info(source),
    }
}

#[tokio::test]
async fn load_models_scans_rows_then_router_entries() {
    let (_fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_ProxyModelTable\" \
             (\"model_id\", \"model_name\", \"litellm_params\", \"model_info\", \"created_by\", \"updated_by\") \
             VALUES \
             ('db-a', 'stored-a', '{}'::jsonb, \
              '{\"ptu_count\": 2, \"cost_per_ptu_per_hour\": 0.5, \"team_id\": \"team-1\", \"ptu_effective_from\": \"2026-01-01T00:00:00Z\"}'::jsonb, 't', 't'), \
             ('db-b', 'stored-b', '{}'::jsonb, '{\"other\": 1}'::jsonb, 't', 't'), \
             ('', 'anonymous', '{}'::jsonb, \
              '{\"ptu_count\": 1, \"cost_per_ptu_per_hour\": 1, \"team_id\": \"team-1\", \"ptu_effective_from\": \"2026-01-01T00:00:00Z\"}'::jsonb, 't', 't')",
        )
        .await
        .unwrap();
    let text = Json(
        r#"{"ptu_count": 1, "cost_per_ptu_per_hour": 1, "team_id": "team-1", "ptu_effective_from": "2026-01-01T00:00:00Z"}"#
            .to_owned(),
    );
    client
        .execute(
            "INSERT INTO \"LiteLLM_ProxyModelTable\" \
             (\"model_id\", \"model_name\", \"litellm_params\", \"model_info\", \"created_by\", \"updated_by\") \
             VALUES ('db-text', 'stored-text', '{}'::jsonb, $1, 't', 't')",
            &[&text],
        )
        .await
        .unwrap();
    let terms = r#""ptu_count": 1, "cost_per_ptu_per_hour": 1, "team_id": "team-1", "ptu_effective_from": "2026-01-01T00:00:00Z""#;
    let entries = [
        router_entry(&format!(r#"{{"id": "cfg-a", {terms}}}"#)),
        router_entry(&format!(r#"{{"id": "db-a", {terms}}}"#)),
        router_entry(&format!(
            r#"{{"id": "cfg-clone", "db_model": true, {terms}}}"#
        )),
    ];
    let loaded = load_models(&client, &entries).await.unwrap();
    assert_eq!(loaded.models.len(), 4);
    assert_eq!(loaded.models.last().unwrap().model_id, "cfg-a");
    assert_eq!(
        loaded
            .models
            .iter()
            .filter(|model| model.model_id == "db-a")
            .count(),
        1
    );
    assert_eq!(
        loaded.scanned_ids,
        BTreeSet::from([
            String::new(),
            "cfg-a".to_owned(),
            "db-a".to_owned(),
            "db-b".to_owned(),
            "db-text".to_owned(),
        ])
    );
}
