#![cfg(test)]

use super::sql::{session_upsert_sql, upsert_autorouter_session_sql};
use super::transaction::{AutoRouterTurnTransaction, FIELD_ORDER, TurnFlag, upsert_params};
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, FixedOffset, TimeZone as _, Utc};
use deadpool_postgres::{Client, Pool};
use rust_decimal::Decimal;

fn turn() -> AutoRouterTurnTransaction {
    AutoRouterTurnTransaction {
        api_key: "key".to_owned(),
        session_id: "session".to_owned(),
        router_name: "router".to_owned(),
        router_type: "simple-shuffle".to_owned(),
        model: "model".to_owned(),
        turn_at: Utc
            .with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
            .unwrap()
            .fixed_offset(),
        total_tokens: 9,
        spend: Decimal::new(5, 1),
        saved_spend: Decimal::new(1, 1),
        classifier_cost: Decimal::new(1, 2),
        covered: TurnFlag(true),
        cache_hit: TurnFlag(false),
        cache_ttl_seconds: None,
        cache_touched: TurnFlag(false),
        tier: None,
        baseline_model: None,
        savings_estimated_turns: 0,
        savings_estimated_actual_spend: Decimal::ZERO,
        savings_estimated_saved_spend: Decimal::ZERO,
        user_id: String::new(),
    }
}

#[test]
fn bind_order_matches_all_turn_fields_and_datetime_is_iso() {
    assert_eq!(FIELD_ORDER.len(), 20);
    let params = upsert_params(&turn());
    assert_eq!(params.len(), FIELD_ORDER.len());
}

#[test]
fn key_and_user_updates_share_one_statement_and_skip_empty_identity_rows() {
    let sql = upsert_autorouter_session_sql();
    assert!(sql.contains("WITH key_rollup AS"));
    assert!(sql.contains("WHERE $1::text <> ''"));
    assert!(sql.contains("WHERE $20::text <> ''"));
    assert!(sql.contains("$6::timestamp >= t.last_turn_at"));
    assert!(sql.contains("EXTRACT(EPOCH"));
}

#[test]
fn separate_session_sql_uses_user_identity_only_for_user_scope() {
    assert!(session_upsert_sql(false).contains("WHERE $1::text <> ''"));
    assert!(session_upsert_sql(true).contains("WHERE $20::text <> ''"));
}

#[derive(clap::Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) async fn database() -> (PostgresFixture, Pool) {
    use clap::Parser as _;
    use secrecy::ExposeSecret as _;
    let url = Environment::parse_from(["turn-tests"])
        .url
        .map(secrecy::SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let manager = deadpool_postgres::Manager::new(
        fixture.options.url.expose_secret().parse().unwrap(),
        tokio_postgres::NoTls,
    );
    let pool = Pool::builder(manager).max_size(3).build().unwrap();
    let client = pool.get().await.unwrap();
    client.batch_execute(concat!(
        include_str!("../../migrations/20260805000000_add_autorouter_session_rollup/migration.sql"),
        include_str!("../../migrations/20260807000000_add_autorouter_session_tier_turns/migration.sql"),
        include_str!("../../migrations/20260907000000_add_autorouter_classifier_cost/migration.sql"),
        include_str!("../../migrations/20260910000000_add_autorouter_session_baseline_models/migration.sql"),
        include_str!("../../migrations/20260915000000_add_autorouter_savings_estimate_coverage/migration.sql"),
        include_str!("../../migrations/20260919000000_add_autorouter_user_session_rollup/migration.sql"),
    )).await.unwrap();
    (fixture, pool)
}

#[tokio::test]
async fn postgres_initial_turn_updates_both_scopes() {
    let (fixture, pool) = database().await;
    let mut client = pool.get().await.unwrap();
    let transaction = client.transaction().await.unwrap();
    let mut input = turn();
    input.user_id = "user".to_owned();
    input.cache_hit = TurnFlag(true);
    super::write::write_autorouter_turn(&transaction, &input)
        .await
        .unwrap();
    transaction.commit().await.unwrap();
    let result = client.query_one("SELECT turns, first_visit_turns, same_model_turns, spend::numeric, first_visit_hits FROM \"LiteLLM_AutoRouterUserSession\"", &[]).await.unwrap();
    assert_eq!(result.get::<_, i32>(0), 1_i32);
    assert_eq!(result.get::<_, i32>(1), 1_i32);
    assert_eq!(result.get::<_, i32>(2), 0_i32);
    assert_eq!(result.get::<_, Decimal>(3), input.spend);
    assert_eq!(result.get::<_, i32>(4), 1_i32);
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn postgres_order_ties_cache_ttl_and_tier_counters() {
    let (fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    let mut input = turn();
    input.user_id = "user".to_owned();
    input.cache_ttl_seconds = Some(300);
    input.tier = Some("low".to_owned());
    input.baseline_model = Some("base".to_owned());
    input.savings_estimated_turns = 1;
    let start = input.turn_at;
    write_ordered_trace(&client, &mut input).await;
    input.turn_at = start - chrono::Duration::seconds(1);
    input.model = "b".to_owned();
    input.cache_ttl_seconds = Some(3600);
    input.router_type = "different".to_owned();
    input.savings_estimated_turns = 2;
    input.cache_hit = TurnFlag(false);
    super::write::write_autorouter_turn(&client, &input)
        .await
        .unwrap();
    check_rollup(&client, start, input.turn_at).await;
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

async fn check_rollup(
    client: &Client,
    start: DateTime<FixedOffset>,
    oldest: DateTime<FixedOffset>,
) {
    let rows = client.query_one("SELECT last_model, first_turn_at, last_turn_at, (models->'b'->>'ttl')::int, (models->'b'->>'at')::bigint, spend::numeric FROM \"LiteLLM_AutoRouterSession\"", &[]).await.unwrap();
    assert_eq!(rows.get::<_, String>(0), "a");
    assert_eq!(
        rows.get::<_, chrono::NaiveDateTime>(1),
        oldest.naive_local()
    );
    assert_eq!(
        rows.get::<_, chrono::NaiveDateTime>(2),
        start
            .checked_add_signed(chrono::Duration::seconds(301))
            .unwrap()
            .naive_local()
    );
    assert_eq!(rows.get::<_, i32>(3), 300_i32);
    assert_eq!(
        rows.get::<_, i64>(4),
        start.timestamp().checked_add(10).unwrap()
    );
    assert_eq!(rows.get::<_, Decimal>(5), Decimal::new(35, 1));

    let row = client.query_one("SELECT ARRAY[turns, unordered_turns, covered_turns, cache_hits, same_model_turns, same_model_hits, first_visit_turns, return_turns, return_expired_misses, return_within_ttl_misses, ttl_5m_turns, ttl_1h_turns, classifier_cost_recorded_turns, (tier_turns->>'low')::int, (baseline_models->>'base')::int, (savings_estimated_baseline_models->>'base')::int, savings_estimated_turns] FROM \"LiteLLM_AutoRouterSession\"", &[]).await.unwrap();
    assert_eq!(
        row.get::<_, Vec<i32>>(0),
        vec![
            7_i32, 1_i32, 7_i32, 1_i32, 1_i32, 1_i32, 2_i32, 3_i32, 1_i32, 2_i32, 6_i32, 1_i32,
            7_i32, 6_i32, 7_i32, 6_i32, 8_i32
        ]
    );
    let equal = client.query_one("SELECT (to_jsonb(k) = to_jsonb(u) - 'user_id') FROM \"LiteLLM_AutoRouterSession\" k JOIN \"LiteLLM_AutoRouterUserSession\" u USING (api_key, session_id, router_name)", &[]).await.unwrap();
    assert!(equal.get::<_, bool>(0));
}

#[rstest::rstest]
#[case::both_empty("", "", 0, 0)]
#[case::user_only("", "user", 0, 1)]
#[case::key_only("key", "", 1, 0)]
#[tokio::test]
async fn postgres_empty_identities(
    #[case] key: &str,
    #[case] user: &str,
    #[case] keys: i64,
    #[case] users: i64,
) {
    let (fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    let mut input = turn();
    input.api_key = key.to_owned();
    input.user_id = user.to_owned();
    super::write::write_autorouter_turn(&client, &input)
        .await
        .unwrap();
    let result = client.query_one("SELECT (SELECT count(*) FROM \"LiteLLM_AutoRouterSession\"), (SELECT count(*) FROM \"LiteLLM_AutoRouterUserSession\")", &[]).await.unwrap();
    assert_eq!(
        (result.get::<_, i64>(0), result.get::<_, i64>(1)),
        (keys, users)
    );
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn postgres_concurrent_collisions_and_atomic_rollback() {
    let (fixture, pool) = database().await;
    let first = pool.get().await.unwrap();
    let second = pool.get().await.unwrap();
    let mut input = turn();
    input.user_id = "user".to_owned();
    let (left, right) = tokio::join!(
        super::write::write_autorouter_turn(&first, &input),
        super::write::write_autorouter_turn(&second, &input)
    );
    left.unwrap();
    right.unwrap();
    let result = first
        .query_one(
            "SELECT turns, same_model_turns, total_tokens FROM \"LiteLLM_AutoRouterSession\"",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        (
            result.get::<_, i32>(0),
            result.get::<_, i32>(1),
            result.get::<_, i64>(2)
        ),
        (2_i32, 1_i32, 18_i64)
    );
    assert_atomic_failure(&first, &input).await;
    drop((first, second));
    pool.close();
    fixture.close_async().await.unwrap();
}

async fn assert_atomic_failure(first: &Client, input: &AutoRouterTurnTransaction) {
    first
        .batch_execute("ALTER TABLE \"LiteLLM_AutoRouterUserSession\" ADD CHECK (turns < 3)")
        .await
        .unwrap();
    super::write::write_autorouter_turn(first, input)
        .await
        .unwrap_err();
    let remaining = first
        .query_one("SELECT turns FROM \"LiteLLM_AutoRouterSession\"", &[])
        .await
        .unwrap();
    assert_eq!(remaining.get::<_, i32>(0), 2_i32);
}

async fn write_ordered_trace(client: &Client, input: &mut AutoRouterTurnTransaction) {
    let start = input.turn_at;
    for (offset, model, touched, hit) in [
        (0, "a", true, false),
        (10, "b", true, false),
        (300, "a", false, false),
        (300, "b", false, false),
        (301, "a", false, false),
        (301, "a", true, true),
    ] {
        input.turn_at = start
            .checked_add_signed(chrono::Duration::seconds(offset))
            .unwrap();
        input.model = model.to_owned();
        input.cache_touched = TurnFlag(touched);
        input.cache_hit = TurnFlag(hit);
        super::write::write_autorouter_turn(client, input)
            .await
            .unwrap();
    }
}
