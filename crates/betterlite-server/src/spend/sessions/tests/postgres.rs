use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use super::super::cursor::SessionCursor;
use super::super::grouped::sql::{GroupedPage, count_query, page_query, representative_query};
use super::super::grouped::{
    GroupedSessionRequest, SessionLogsPage, ui_session_grouped_spend_logs,
};
use super::super::ports::SessionsStore;
use super::super::store::PostgresSessions;
use super::support::{call, internal, json, state};

// only the columns the session reads touch, with the types the real table declares
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
    let url = Environment::parse_from(["spend-session-tests"])
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

fn sessions_store(pool: &Pool) -> PostgresSessions {
    PostgresSessions {
        store: PostgresStore {
            pool: Some(pool.clone()),
        },
    }
}

async fn execute(pool: &Pool, sql: &str) {
    let client = pool.get().await.unwrap();
    client.batch_execute(sql).await.unwrap();
}

fn page_ids(page: &SessionLogsPage) -> Vec<&str> {
    page.page
        .data
        .iter()
        .map(|row| row.request_id.as_str())
        .collect()
}

fn request_id(row: &JsonData) -> String {
    let JsonData::Object(fields) = row else {
        panic!("the row is not an object");
    };
    let Some(JsonData::String(id)) = fields.get("request_id") else {
        panic!("the row names no request id");
    };
    id.clone()
}

const PRIORITY_ROWS: &str = r#"
INSERT INTO "LiteLLM_SpendLogs"
(request_id, call_type, api_key, "startTime", "endTime", session_id, "user") VALUES
('plain', 'acompletion', 'k1', '2024-01-01 00:00:00', '2024-01-01 00:00:01', 's1', 'alice'),
('agent-early', 'asend_message', 'k1', '2024-01-01 01:00:00', '2024-01-01 01:00:05', 's1', 'alice'),
('agent-late', 'asend_message', 'k1', '2024-01-01 00:30:00', '2024-01-01 02:00:00', 's1', 'alice'),
('mcp-new', 'call_mcp_tool', 'k1', '2024-01-01 03:00:00', '2024-01-01 03:00:01', 's1', 'alice');
"#;

#[tokio::test]
async fn a_representative_is_the_latest_agent_outcome() {
    let (_fixture, pool) = database().await;
    execute(&pool, PRIORITY_ROWS).await;
    let store = sessions_store(&pool);
    let keys = [(String::from("s1"), String::from("k1"))];
    let rows = store
        .session_representatives(&representative_query(&[], &[], &keys))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].request_id, "agent-late");
}

const TIE_ROWS: &str = r#"
INSERT INTO "LiteLLM_SpendLogs"
(request_id, call_type, api_key, "startTime", "endTime", session_id) VALUES
('llm-old', 'acompletion', 'k1', '2024-01-01 00:00:00', '2024-01-01 00:00:01', 's1'),
('mcp-new', 'call_mcp_tool', 'k1', '2024-01-02 00:00:00', '2024-01-02 00:00:01', 's1'),
('tie-b', 'acompletion', 'k2', '2024-01-03 00:00:00', '2024-01-03 00:00:01', 's2'),
('tie-a', 'acompletion', 'k2', '2024-01-03 00:00:00', '2024-01-03 00:00:01', 's2');
"#;

#[tokio::test]
async fn a_non_agent_representative_skips_the_newer_mcp_tool() {
    let (_fixture, pool) = database().await;
    execute(&pool, TIE_ROWS).await;
    let store = sessions_store(&pool);
    let keys = [(String::from("s1"), String::from("k1"))];
    let rows = store
        .session_representatives(&representative_query(&[], &[], &keys))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].request_id, "llm-old");
}

#[tokio::test]
async fn an_equal_representative_tie_breaks_on_the_request_id() {
    let (_fixture, pool) = database().await;
    execute(&pool, TIE_ROWS).await;
    let store = sessions_store(&pool);
    let keys = [(String::from("s2"), String::from("k2"))];
    let rows = store
        .session_representatives(&representative_query(&[], &[], &keys))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].request_id, "tie-a");
}

const FALLBACK_ROWS: &str = r#"
INSERT INTO "LiteLLM_SpendLogs"
(request_id, call_type, api_key, "startTime", "endTime", session_id) VALUES
('req-null', 'acompletion', 'k1', '2024-01-02 00:00:00', '2024-01-02 00:00:01', NULL),
('req-empty', 'acompletion', 'k1', '2024-01-01 00:00:00', '2024-01-01 00:00:01', '');
"#;

#[tokio::test]
async fn a_missing_session_id_groups_under_its_request_id() {
    let (_fixture, pool) = database().await;
    execute(&pool, FALLBACK_ROWS).await;
    let store = sessions_store(&pool);
    let request = GroupedSessionRequest {
        conditions: &[],
        params: Vec::new(),
        page: 1,
        page_size: 10,
        descending: true,
        session_cursor: None,
    };
    let page = ui_session_grouped_spend_logs(&store, None, &request)
        .await
        .unwrap();
    assert_eq!(page_ids(&page), ["req-null", "req-empty"]);
    assert_eq!(page.page.total, 2);
    assert!(!page.has_more);
}

const TUPLE_ROWS: &str = r#"
INSERT INTO "LiteLLM_SpendLogs"
(request_id, call_type, api_key, "startTime", "endTime", session_id) VALUES
('a-one', 'acompletion', 'k1', '2024-01-05 00:00:00', '2024-01-05 00:00:01', 'a'),
('a-two', 'acompletion', 'k2', '2024-01-05 00:00:00', '2024-01-05 00:00:01', 'a'),
('b-one', 'acompletion', 'k1', '2024-01-05 00:00:00', '2024-01-05 00:00:01', 'b');
"#;

#[tokio::test]
async fn a_cursor_continues_after_the_whole_group_tuple() {
    let (_fixture, pool) = database().await;
    execute(&pool, TUPLE_ROWS).await;
    let store = sessions_store(&pool);
    let first = GroupedPage {
        conditions: &[],
        params: Vec::new(),
        cursor: None,
        descending: true,
        page_limit: 5,
        offset: 0,
    };
    let groups = store.session_groups(&page_query(&first)).await.unwrap();
    let keys: Vec<(&str, &str)> = groups
        .iter()
        .map(|row| (row.session_key.as_str(), row.api_key.as_str()))
        .collect();
    assert_eq!(keys, [("b", "k1"), ("a", "k2"), ("a", "k1")]);
    let cursor = SessionCursor {
        last_activity: groups[1].last_activity.clone(),
        session_key: String::from("a"),
        api_key_hash: String::from("k2"),
    };
    let next = GroupedPage {
        cursor: Some(cursor),
        ..first
    };
    let continued = store.session_groups(&page_query(&next)).await.unwrap();
    assert_eq!(continued.len(), 1);
    assert_eq!(continued[0].session_key, "a");
    assert_eq!(continued[0].api_key, "k1");
}

const FOUR_SESSIONS: &str = r#"
INSERT INTO "LiteLLM_SpendLogs"
(request_id, call_type, api_key, "startTime", "endTime", session_id) VALUES
('r-s1', 'acompletion', 'k1', '2024-01-01 00:00:00', '2024-01-01 00:00:01', 's1'),
('r-s2', 'acompletion', 'k1', '2024-01-02 00:00:00', '2024-01-02 00:00:01', 's2'),
('r-s3', 'acompletion', 'k1', '2024-01-03 00:00:00', '2024-01-03 00:00:01', 's3'),
('r-s4', 'acompletion', 'k1', '2024-01-04 00:00:00', '2024-01-04 00:00:01', 's4');
"#;

#[tokio::test]
async fn a_cursor_page_continues_after_the_reported_position() {
    let (_fixture, pool) = database().await;
    execute(&pool, FOUR_SESSIONS).await;
    let store = sessions_store(&pool);
    let first = GroupedSessionRequest {
        conditions: &[],
        params: Vec::new(),
        page: 1,
        page_size: 2,
        descending: true,
        session_cursor: None,
    };
    let page = ui_session_grouped_spend_logs(&store, None, &first)
        .await
        .unwrap();
    assert_eq!(page_ids(&page), ["r-s4", "r-s3"]);
    assert!(page.has_more);
    let cursor = page
        .next_session_cursor
        .expect("a page with more sessions names its cursor");
    let second = GroupedSessionRequest {
        session_cursor: Some(cursor.as_str()),
        page: 2,
        ..first
    };
    let page = ui_session_grouped_spend_logs(&store, None, &second)
        .await
        .unwrap();
    assert_eq!(page_ids(&page), ["r-s2", "r-s1"]);
    assert!(!page.has_more);
    assert_eq!(page.page.total, 4);
    assert_eq!(page.next_session_cursor, None);
}

const CAP_ROWS: &str = r#"
INSERT INTO "LiteLLM_SpendLogs"
(request_id, call_type, api_key, "startTime", "endTime", session_id)
SELECT 'r-' || i, 'acompletion', 'k1',
       '2024-01-01 00:00:00', '2024-01-01 00:00:01', 's-' || i
FROM generate_series(1, 10001) AS i;
"#;

#[tokio::test]
async fn a_counted_page_stops_at_the_cap() {
    let (_fixture, pool) = database().await;
    execute(&pool, CAP_ROWS).await;
    let store = sessions_store(&pool);
    let raw = store
        .session_group_count(&count_query(&[], &[]))
        .await
        .unwrap();
    assert_eq!(raw, 10_001);
    let request = GroupedSessionRequest {
        conditions: &[],
        params: Vec::new(),
        page: 5_001,
        page_size: 2,
        descending: true,
        session_cursor: None,
    };
    let page = ui_session_grouped_spend_logs(&store, None, &request)
        .await
        .unwrap();
    assert!(page.page.data.is_empty());
    assert!(page.page.total_is_capped);
    assert_eq!(page.page.total, 10_000);
    assert!(!page.has_more);
}

const EXPANSION_ROWS: &str = r#"
INSERT INTO "LiteLLM_SpendLogs"
(request_id, call_type, api_key, spend, "startTime", "endTime", model, "user", metadata, request_tags, session_id) VALUES
('alice-old', 'acompletion', 'k1', 0.5, '2024-01-01 00:00:00', '2024-01-01 00:00:02', 'gpt-4o', 'alice', '{"status": "success"}', '["team-a"]', 's1'),
('alice-new', 'acompletion', 'k1', 0.25, '2024-01-02 00:00:00', '2024-01-02 00:00:03', 'gpt-4o', 'alice', '{"status": "success"}', '["team-a"]', 's1'),
('bob', 'acompletion', 'k1', 1, '2024-01-03 00:00:00', '2024-01-03 00:00:01', 'gpt-4o', 'bob', '{}', '[]', 's1');
"#;

#[tokio::test]
async fn the_expansion_route_scopes_a_non_admin_to_their_rows() {
    let (_fixture, pool) = database().await;
    execute(&pool, EXPANSION_ROWS).await;
    let store: Arc<dyn SessionsStore> = Arc::new(sessions_store(&pool));
    let uri = "/spend/logs/session/ui?session_id=s1";
    let reply = call(state(Some(store), None), uri, Some(&internal("alice"))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let JsonData::Object(body) = reply.json() else {
        panic!("the body is not an object");
    };
    assert_eq!(body.get("total"), Some(&json("2")));
    assert_eq!(body.get("page"), Some(&json("1")));
    assert_eq!(body.get("page_size"), Some(&json("50")));
    assert_eq!(body.get("total_pages"), Some(&json("1")));
    let Some(JsonData::Array(data)) = body.get("data") else {
        panic!("data is not an array");
    };
    let ids: Vec<String> = data.iter().map(request_id).collect();
    assert_eq!(ids, ["alice-new", "alice-old"]);
}

#[tokio::test]
async fn the_expansion_route_decodes_metadata_and_keeps_tags_text() {
    let (_fixture, pool) = database().await;
    execute(&pool, EXPANSION_ROWS).await;
    let store: Arc<dyn SessionsStore> = Arc::new(sessions_store(&pool));
    let uri = "/spend/logs/session/ui?session_id=s1";
    let reply = call(state(Some(store), None), uri, Some(&internal("alice"))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let JsonData::Object(body) = reply.json() else {
        panic!("the body is not an object");
    };
    assert_eq!(body.get("total"), Some(&json("2")));
    let Some(JsonData::Array(data)) = body.get("data") else {
        panic!("data is not an array");
    };
    let Some(JsonData::Object(row)) = data.first() else {
        panic!("the page holds no row");
    };
    assert_eq!(row.get("metadata"), Some(&json("{\"status\":\"success\"}")));
    assert_eq!(row.get("request_tags"), Some(&json("\"[\\\"team-a\\\"]\"")));
    assert_eq!(row.get("litellm_call_id"), None);
    assert_eq!(row.get("session_total_count"), None);
}
