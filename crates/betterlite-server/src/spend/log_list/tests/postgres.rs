use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use super::super::authz::{Authorization, ScopeClause};
use super::super::conditions::{ConditionInputs, build_conditions};
use super::super::order::order_clause;
use super::super::page::{PageWindow, count_query, page_query};
use super::super::store::{LogListStore as _, PostgresLogList};
use crate::spend::log_query::decode_spend_log_query;
use crate::spend::log_query::prepare::SpendLogRoute;
use crate::spend::log_query::validate::SpendLogChoices;
use crate::spend::log_results::projection::SpendLogListRow;
use crate::spend::sessions::grouped::sql::where_clause;
use crate::spend::sessions::params::SessionParam;

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
    model_id text,
    model_group text,
    custom_llm_provider text,
    api_base text,
    "user" text NOT NULL DEFAULT '',
    metadata jsonb DEFAULT '{}',
    cache_hit text,
    cache_key text,
    request_tags jsonb DEFAULT '[]',
    team_id text,
    organization_id text,
    end_user text,
    requester_ip_address text,
    session_id text,
    status text,
    mcp_namespaced_tool_name text,
    agent_id text,
    litellm_call_id text,
    request_duration_ms integer
);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["spend-log-list-tests"])
        .url
        .map(SecretString::from);
    let fixture =
        PostgresFixture::provision_with_statements(url.as_ref(), vec![String::from(SCHEMA)])
            .resolve()
            .await
            .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    (fixture, pool)
}

fn log_list(pool: &Pool) -> PostgresLogList {
    PostgresLogList {
        store: PostgresStore {
            pool: Some(pool.clone()),
        },
    }
}

async fn execute(pool: &Pool, sql: &str) {
    let client = pool.get().await.unwrap();
    client.batch_execute(sql).await.unwrap();
}

fn prisma_text(value: Option<JsonData>) -> String {
    let Some(JsonData::String(text)) = value else {
        panic!("the column is not the text the Prisma serialiser returns");
    };
    text
}

const ROWS: &str = r#"
INSERT INTO "LiteLLM_SpendLogs"
(request_id, call_type, api_key, "startTime", "endTime", "user", team_id, spend, metadata, request_tags) VALUES
('r1', 'acompletion', 'k1', '2024-01-01 00:00:00', '2024-01-01 00:00:01', 'alice', NULL, 1.0, '{"a":1}', '["t1"]'),
('r2', 'acompletion', 'k1', '2024-01-02 00:00:00', '2024-01-02 00:00:01', 'alice', NULL, 2.0, '{}', '[]'),
('r3', 'acompletion', 'k2', '2024-01-03 00:00:00', '2024-01-03 00:00:01', 'bob', NULL, 3.0, '{}', '[]');
"#;

#[tokio::test]
async fn the_page_read_returns_prisma_text_jsonb_in_the_requested_order() {
    let (_fixture, pool) = database().await;
    execute(&pool, ROWS).await;
    let store = log_list(&pool);
    let params = vec![SessionParam::Text(String::from("alice"))];
    let joined = where_clause(&[String::from("\"user\" = $1")]);
    let count = store
        .count_matches(&count_query(&joined, false, &params))
        .await
        .unwrap();
    assert_eq!(count, 2);
    let window = PageWindow {
        page_size: 1,
        offset: 1,
    };
    let rows = store
        .list_rows(&page_query(
            &joined,
            "\"startTime\" DESC",
            false,
            &params,
            window,
        ))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].request_id, "r1");
    let metadata = prisma_text(rows[0].metadata.clone());
    assert!(metadata.contains("\"a\": 1"));
    let tags = prisma_text(rows[0].request_tags.clone());
    assert!(tags.contains("t1"));
}

const GROUPED_ROWS: &str = r#"
INSERT INTO "LiteLLM_SpendLogs"
(request_id, call_type, api_key, "startTime", "endTime", "user", session_id, status) VALUES
('s1-old-fail', 'acompletion', 'k1', '2024-01-01 00:00:00', '2024-01-01 00:00:01', 'alice', 's1', 'failure'),
('s1-new-ok', 'acompletion', 'k1', '2024-01-02 00:00:00', '2024-01-02 00:00:01', 'alice', 's1', 'success'),
('s2-old-ok', 'acompletion', 'k1', '2024-01-01 00:00:00', '2024-01-01 00:00:01', 'alice', 's2', 'success'),
('s2-new-fail', 'acompletion', 'k1', '2024-01-02 00:00:00', '2024-01-02 00:00:01', 'alice', 's2', 'failure');
"#;

const GROUPED_QUERY: &str = "start_date=2023-12-01%2000:00:00&end_date=2024-02-01%2000:00:00&status_filter=failure&group_by_session=true";

#[tokio::test]
async fn the_grouped_status_read_keeps_only_sessions_whose_representative_matched() {
    let (_fixture, pool) = database().await;
    execute(&pool, GROUPED_ROWS).await;
    let store = log_list(&pool);
    let query = decode_spend_log_query(Some(GROUPED_QUERY));
    let prepared = query.prepare(SpendLogRoute::Ui).unwrap();
    let choices = query.choices().unwrap();
    let built = build_conditions(&ConditionInputs {
        query: &query,
        prepared: &prepared,
        choices: &choices,
        authorization: &Authorization {
            scope: ScopeClause::None,
        },
    });
    let joined = where_clause(&built.conditions);
    let total = store
        .count_matches(&count_query(&joined, true, &built.params))
        .await
        .unwrap();
    assert_eq!(total, 1);
    let rows = grouped_page_rows(
        &store,
        &joined,
        &built.params,
        &choices,
        built.request_id_index,
    )
    .await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].request_id, "s2-new-fail");
}

async fn grouped_page_rows(
    store: &PostgresLogList,
    joined: &str,
    params: &[SessionParam],
    choices: &SpendLogChoices,
    request_id_index: Option<usize>,
) -> Vec<SpendLogListRow> {
    let order = order_clause(choices, request_id_index, true);
    let window = PageWindow {
        page_size: 10,
        offset: 0,
    };
    store
        .list_rows(&page_query(joined, &order, true, params, window))
        .await
        .unwrap()
}
