use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_models::key_digest::hashing::hash_token;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use super::support::{decimal, json};
use crate::spend::log_payload::daily_summary::{SummaryFilters, daily_summary_sql};
use crate::spend::log_results::filters::LegacyFilters;
use crate::spend::log_results::query::decode_legacy_spend_query;
use crate::spend::log_results::rows::spend_log_rows_query;
use crate::spend::log_results::session::{
    SessionAggregate, SessionAggregateReader as _, SessionLookup,
};
use crate::spend::log_results::store::{LogResultsStore as _, PostgresLogResults};

// only the columns these reads touch, with the timestamp types the real table declares
const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_SpendLogs" (
    request_id text PRIMARY KEY,
    call_type text NOT NULL DEFAULT '',
    api_key text NOT NULL DEFAULT '',
    spend double precision NOT NULL DEFAULT 0.0,
    total_tokens integer NOT NULL DEFAULT 0,
    prompt_tokens integer NOT NULL DEFAULT 0,
    completion_tokens integer NOT NULL DEFAULT 0,
    "startTime" timestamp(3) NOT NULL,
    "endTime" timestamp(3) NOT NULL,
    "completionStartTime" timestamp(3),
    model text NOT NULL DEFAULT '',
    "user" text NOT NULL DEFAULT '',
    session_id text,
    litellm_call_id text,
    cache_hit text,
    request_duration_ms integer,
    created_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP
);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["spend-log-results-tests"])
        .url
        .map(SecretString::from);
    let fixture =
        PostgresFixture::provision_with_statements(url.as_ref(), vec![String::from(SCHEMA)])
            .resolve()
            .await
            .unwrap();
    let settings = fixture.options.url.expose_secret().parse().unwrap();
    let manager = Manager::new(settings, NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    (fixture, pool)
}

fn log_results_store(pool: &Pool) -> PostgresLogResults {
    PostgresLogResults {
        store: PostgresStore {
            pool: Some(pool.clone()),
        },
    }
}

async fn execute(pool: &Pool, sql: &str) {
    let client = pool.get().await.unwrap();
    client.batch_execute(sql).await.unwrap();
}

fn legacy_rows_sql() -> String {
    let key = hash_token("sk-live");
    format!(
        r#"
INSERT INTO "LiteLLM_SpendLogs"
(request_id, call_type, api_key, spend, "startTime", "endTime", model, "user", litellm_call_id) VALUES
('first', 'acompletion', 'hashed', 0.5, '2024-01-02 00:00:00', '2024-01-02 00:00:01', 'gpt-4o', 'alice', NULL),
('last', 'acompletion', 'hashed', 0.5, '2024-01-03 00:00:00', '2024-01-03 00:00:01', 'gpt-4o', 'alice', NULL),
('after', 'acompletion', 'hashed', 0.5, '2024-01-03 00:00:01', '2024-01-03 00:00:02', 'gpt-4o', 'alice', NULL),
('before', 'acompletion', 'hashed', 0.5, '2024-01-01 23:59:59', '2024-01-02 00:00:00', 'gpt-4o', 'alice', NULL),
('bob', 'acompletion', 'hashed', 0.5, '2024-01-02 12:00:00', '2024-01-02 12:00:01', 'gpt-4o', 'bob', NULL),
('by-call', 'acompletion', 'hashed', 0.5, '2024-01-02 06:00:00', '2024-01-02 06:00:01', 'gpt-4o', 'alice', 'call-1'),
('by-key', 'acompletion', '{key}', 0.5, '2024-01-02 07:00:00', '2024-01-02 07:00:01', 'gpt-4o', 'alice', NULL);
"#
    )
}

#[tokio::test]
async fn legacy_rows_come_newest_first_with_utc_midnight_bounds() {
    let (_fixture, pool) = database().await;
    execute(&pool, &legacy_rows_sql()).await;
    let store = log_results_store(&pool);
    let window = decode_legacy_spend_query(b"start_date=2024-01-02&end_date=2024-01-03").unwrap();
    let window_filters = LegacyFilters::resolve(&window, Some(String::from("alice"))).unwrap();
    let rows = store
        .spend_logs(&spend_log_rows_query(&window_filters))
        .await
        .unwrap();
    let ids: Vec<&str> = rows.iter().map(|row| row.request_id.as_str()).collect();
    assert_eq!(ids, ["last", "by-key", "by-call", "first"]);
    let text = serde_json::to_string(&rows[0]).unwrap();
    let JsonData::Object(fields) = json(&text) else {
        panic!("the row is not an object");
    };
    assert_eq!(
        fields.get("startTime"),
        Some(&json("\"2024-01-03T00:00:00Z\""))
    );
}

#[tokio::test]
async fn a_request_id_also_matches_the_litellm_call_id() {
    let (_fixture, pool) = database().await;
    execute(&pool, &legacy_rows_sql()).await;
    let store = log_results_store(&pool);
    let lookup = decode_legacy_spend_query(b"request_id=call-1&summarize=false").unwrap();
    let lookup_filters = LegacyFilters::resolve(&lookup, None).unwrap();
    let rows = store
        .spend_logs(&spend_log_rows_query(&lookup_filters))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].request_id, "by-call");
}

#[tokio::test]
async fn an_sk_api_key_is_hashed_before_it_reaches_the_query() {
    let (_fixture, pool) = database().await;
    execute(&pool, &legacy_rows_sql()).await;
    let store = log_results_store(&pool);
    let key = decode_legacy_spend_query(b"api_key=sk-live&summarize=false").unwrap();
    let key_filters = LegacyFilters::resolve(&key, None).unwrap();
    let rows = store
        .spend_logs(&spend_log_rows_query(&key_filters))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].request_id, "by-key");
}

const SUMMARY_ROWS: &str = r#"
INSERT INTO "LiteLLM_SpendLogs"
(request_id, call_type, api_key, spend, "startTime", "endTime", model, "user") VALUES
('sum-a', 'acompletion', 'k1', 0.5, '2024-03-01 10:00:00', '2024-03-01 10:00:01', 'gpt-4o', 'alice'),
('sum-b', 'acompletion', 'k1', 0.25, '2024-03-01 11:00:00', '2024-03-01 11:00:01', 'gpt-4o', 'alice'),
('sum-c', 'acompletion', 'k1', 2, '2024-03-02 00:00:00', '2024-03-02 00:00:01', 'gpt-4o', 'alice'),
('sum-d', 'acompletion', 'k1', 4, '2024-03-03 00:00:00', '2024-03-03 00:00:01', 'gpt-4o', 'alice');
"#;

#[tokio::test]
async fn the_daily_summary_groups_each_day_through_the_inclusive_end() {
    let (_fixture, pool) = database().await;
    execute(&pool, SUMMARY_ROWS).await;
    let store = log_results_store(&pool);
    let query = daily_summary_sql(
        "2024-03-01T00:00:00+00:00",
        "2024-03-02T00:00:00+00:00",
        SummaryFilters {
            api_key: None,
            request_id: None,
            user_id: None,
        },
    );
    let rows = store.daily_summary(&query).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].day, "2024-03-01");
    assert_eq!(rows[0].spend.0, decimal("0.75"));
    assert_eq!(rows[0].api_key.as_deref(), Some("k1"));
    assert_eq!(rows[0].user.as_deref(), Some("alice"));
    assert_eq!(rows[0].model, "gpt-4o");
    assert_eq!(rows[1].day, "2024-03-02");
    assert_eq!(rows[1].spend.0, decimal("2"));
}

const AGGREGATE_ROWS: &str = r#"
INSERT INTO "LiteLLM_SpendLogs"
(request_id, call_type, api_key, spend, total_tokens, prompt_tokens, completion_tokens,
 "startTime", "endTime", model, "user", session_id, cache_hit, request_duration_ms) VALUES
('s1a', 'acompletion', 'k1', 0.5, 30, 10, 20, '2024-02-01 00:00:00', '2024-02-01 00:00:02.500', 'gpt-4o', 'alice', 's1', 'True', NULL),
('s1b', 'call_mcp_tool', 'k1', 0.25, 0, 0, 0, '2024-02-01 00:01:00', '2024-02-01 00:01:01', 'gpt-4o', 'alice', 's1', '', NULL),
('s1c', 'asend_message', 'k1', 0.25, 5, 1, 2, '2024-02-01 00:02:00', '2024-02-01 00:02:03', 'gpt-4o', 'alice', 's1', 'false', 700),
-- a batch call is neither MCP nor agent, so it counts as an LLM call
('s1e', 'batch', 'k1', 0, 0, 0, 0, '2024-02-01 00:04:00', '2024-02-01 00:04:01', 'gpt-4o', 'alice', 's1', '', NULL),
('s1d', 'acompletion', 'k2', 9, 0, 0, 0, '2024-02-01 00:03:00', '2024-02-01 00:03:01', 'gpt-4o', 'bob', 's1', 'False', 0);

INSERT INTO "LiteLLM_SpendLogs"
(request_id, call_type, api_key, spend, "startTime", "endTime", model, "user", session_id)
SELECT 'model-' || lpad(i::text, 2, '0'), 'acompletion', 'k1', 0,
       '2024-02-02 00:00:00', '2024-02-02 00:00:01', 'm' || lpad(i::text, 2, '0'), 'alice', 's2'
FROM generate_series(1, 11) AS i;

INSERT INTO "LiteLLM_SpendLogs"
(request_id, call_type, api_key, spend, "startTime", "endTime", model, "user", session_id)
VALUES ('model-empty', 'acompletion', 'k1', 0, '2024-02-02 00:00:00', '2024-02-02 00:00:01', '', 'alice', 's2');

INSERT INTO "LiteLLM_SpendLogs"
(request_id, call_type, api_key, spend, "startTime", "endTime", model, "user", session_id)
SELECT 'ten-' || lpad(i::text, 2, '0'), 'acompletion', 'k1', 0,
       '2024-02-03 00:00:00', '2024-02-03 00:00:01', 'n' || lpad(i::text, 2, '0'), 'alice', 's3'
FROM generate_series(1, 10) AS i;

INSERT INTO "LiteLLM_SpendLogs"
(request_id, call_type, api_key, spend, "startTime", "endTime", model, "user", session_id)
VALUES
('long-short', 'acompletion', 'k1', 0, '2024-02-04 00:00:00', '2024-02-04 00:00:01', 'short', 'alice', 's4'),
('long-name', 'acompletion', 'k1', 0, '2024-02-04 00:00:01', '2024-02-04 00:00:02', repeat('x', 300), 'alice', 's4');
"#;

fn find<'row>(
    aggregates: &'row [SessionAggregate],
    session_id: &str,
    api_key: &str,
) -> &'row SessionAggregate {
    aggregates
        .iter()
        .find(|aggregate| aggregate.session_id == session_id && aggregate.api_key == api_key)
        .unwrap()
}

#[tokio::test]
async fn session_aggregates_group_by_session_and_key() {
    let (_fixture, pool) = database().await;
    execute(&pool, AGGREGATE_ROWS).await;
    let store = log_results_store(&pool);
    let lookup = SessionLookup {
        session_ids: vec![
            String::from("s1"),
            String::from("s2"),
            String::from("s3"),
            String::from("s4"),
        ],
        api_keys: vec![String::from("k1"), String::from("k2")],
    };
    let aggregates = store.session_aggregates(&lookup).await.unwrap();
    assert_eq!(aggregates.len(), 5);
    let shared = find(&aggregates, "s1", "k1");
    assert_eq!(shared.session_total_count, 4);
    assert_eq!(shared.session_total_spend, decimal("1"));
    assert_eq!(shared.session_total_duration_ms, 5_200);
    assert_eq!(shared.mcp_tool_call_count, 1);
    assert_eq!(shared.mcp_tool_call_spend, decimal("0.25"));
    assert_eq!(shared.session_cache_hit_count, 1);
    assert_eq!(shared.session_llm_count, 2);
    assert_eq!(shared.session_agent_count, 1);
    assert_eq!(shared.session_total_prompt_tokens, 11);
    assert_eq!(shared.session_completion_tokens, 22);
    assert_eq!(shared.session_total_tokens, 35);
    assert_eq!(shared.session_models, ["gpt-4o"]);
    assert!(!shared.session_models_truncated);
    let other_key = find(&aggregates, "s1", "k2");
    assert_eq!(other_key.session_total_count, 1);
    assert_eq!(other_key.session_total_spend, decimal("9"));
}

// eleven names are fetched so ten reach the row, and the flag says an eleventh existed
#[tokio::test]
async fn session_models_stop_at_ten_after_the_name_is_cut() {
    let (_fixture, pool) = database().await;
    execute(&pool, AGGREGATE_ROWS).await;
    let store = log_results_store(&pool);
    let lookup = SessionLookup {
        session_ids: vec![String::from("s2"), String::from("s3"), String::from("s4")],
        api_keys: vec![String::from("k1")],
    };
    let aggregates = store.session_aggregates(&lookup).await.unwrap();
    let eleven = find(&aggregates, "s2", "k1");
    assert_eq!(eleven.session_total_count, 12);
    assert_eq!(
        eleven.session_models,
        [
            "m01", "m02", "m03", "m04", "m05", "m06", "m07", "m08", "m09", "m10"
        ]
    );
    assert!(eleven.session_models_truncated);
    // exactly _SESSION_MODELS_LIMIT models fit, so the page keeps all ten and does not flag them
    let ten = find(&aggregates, "s3", "k1");
    assert_eq!(ten.session_models.len(), 10);
    assert!(!ten.session_models_truncated);
    // a name is kept to _SESSION_MODEL_NAME_MAX_LEN characters
    let cut = find(&aggregates, "s4", "k1");
    assert_eq!(cut.session_models.len(), 2);
    assert_eq!(cut.session_models[0], "short");
    assert_eq!(cut.session_models[1], "x".repeat(256));
}
