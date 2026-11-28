use std::sync::Arc;

use axum::Router;
use axum::http::{HeaderValue, Method, StatusCode, header};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_logging::trace_types::{SpanDetail, SpanErrorPage, Trace, TracePage};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::support::{
    Answer, FakeRuntime, QueryAnswer, ReadAnswer, application, body_text, build_request,
    master_principal, principal, send,
};
use crate::tracing_routes::access::{Tenant, TraceAuth, TraceRole};
use crate::tracing_routes::otlp::OTLP_RETRY_AFTER;
use crate::tracing_routes::query::QueryScope;
use crate::tracing_routes::receiver::TraceRuntime;

const NOW_MS: i64 = 1_700_000_000_000;
const DAY_MS: i64 = 86_400_000;

fn page() -> TracePage {
    serde_json::from_str(r#"{"data":[],"next_cursor":"next"}"#).expect("page")
}

fn trace() -> Trace {
    serde_json::from_str(
        r#"{"summary":{"trace_id":"trace-1","name":"demo","service":"svc","input_preview":"hi","start_time":"2026-01-01T00:00:00Z","duration_ms":1.5,"status":"ok","span_count":1,"agent_count":0,"agent_invocations":0,"llm_calls":1,"tool_calls":0,"error_count":0,"input_tokens":1,"output_tokens":2,"models":["gpt-4o"],"spend":null},"agents":[],"spans":[]}"#,
    )
    .expect("trace")
}

fn span_detail() -> SpanDetail {
    serde_json::from_str(
        r#"{"span_id":"span-1","input":"in","output":"out","input_ui":{"kind":"text","text":"in"},"output_ui":{"kind":"text","text":"out"},"attributes":{}}"#,
    )
    .expect("span detail")
}

fn span_error_page() -> SpanErrorPage {
    serde_json::from_str(
        r#"{"span_id":"span-1","message":"boom","total_chars":4,"next_cursor":null}"#,
    )
    .expect("span error page")
}

fn admin() -> Principal {
    principal(vec![Role::PlatformAdmin], None, None, None, None)
}

fn team_member() -> Principal {
    principal(
        vec![Role::TeamMember],
        Some("team-1"),
        Some("hash-1"),
        None,
        None,
    )
}

fn viewer() -> Principal {
    principal(vec![Role::PlatformViewer], None, None, None, None)
}

fn app(runtime: &Arc<FakeRuntime>, master_key: Option<&str>) -> Router {
    let receiver: Arc<dyn TraceRuntime> = Arc::<FakeRuntime>::clone(runtime);
    application(Some(receiver), master_key)
}

async fn post_traces(
    runtime: &Arc<FakeRuntime>,
    caller: Option<&Principal>,
    content_type: &str,
    body: &[u8],
) -> Answer {
    let headers = [("content-type", content_type)];
    send(
        app(runtime, None),
        build_request(Method::POST, "/v1/traces", caller, &headers, body),
    )
    .await
}

#[tokio::test]
async fn ingest_success_returns_an_empty_object() {
    let runtime = Arc::new(FakeRuntime::new());
    let answer = post_traces(&runtime, Some(&admin()), "application/json", b"{}").await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(body_text(&answer), "{}");
    assert_eq!(
        answer.headers.get(header::CONTENT_TYPE),
        Some(&header_value("application/json"))
    );
    let seen = runtime
        .seen_ingest
        .lock()
        .expect("lock")
        .clone()
        .expect("seen");
    assert_eq!(seen.content_type.as_deref(), Some("application/json"));
    assert_eq!(seen.body_len, 2);
    assert_eq!(
        seen.tenant,
        Tenant {
            team_id: String::new(),
            api_key_hash: String::new(),
            org_id: String::new(),
        }
    );
}

#[tokio::test]
async fn ingest_passes_the_content_encoding_to_the_receiver() {
    let runtime = Arc::new(FakeRuntime::new());
    let headers = [
        ("content-type", "application/json"),
        ("content-encoding", "gzip"),
    ];
    let answer = send(
        app(&runtime, None),
        build_request(Method::POST, "/v1/traces", Some(&admin()), &headers, b"{}"),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    let seen = runtime
        .seen_ingest
        .lock()
        .expect("lock")
        .clone()
        .expect("seen");
    assert_eq!(seen.content_encoding.as_deref(), Some("gzip"));
}

#[tokio::test]
async fn ingest_without_a_content_type_answers_protobuf() {
    let runtime = Arc::new(FakeRuntime::new());
    let answer = send(
        app(&runtime, None),
        build_request(Method::POST, "/v1/traces", Some(&admin()), &[], b""),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    assert!(answer.body.is_empty());
    assert_eq!(
        answer.headers.get(header::CONTENT_TYPE),
        Some(&header_value("application/x-protobuf"))
    );
}

#[tokio::test]
async fn ingest_requires_an_authenticated_principal() {
    let runtime = Arc::new(FakeRuntime::new());
    let answer = post_traces(&runtime, None, "application/json", b"{}").await;
    assert_eq!(answer.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        body_text(&answer),
        r#"{"error":{"message":"the API key is not valid","type":"auth_error","param":"None","code":"401"}}"#
    );
}

#[tokio::test]
async fn ingest_view_only_is_forbidden() {
    let runtime = Arc::new(FakeRuntime::new());
    let answer = post_traces(&runtime, Some(&viewer()), "application/json", b"{}").await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN);
    assert_eq!(
        body_text(&answer),
        r#"{"message": "Not allowed to ingest agent traces"}"#
    );
}

#[tokio::test]
async fn ingest_without_a_receiver_is_not_implemented() {
    let answer = send(
        application(None, None),
        build_request(
            Method::POST,
            "/v1/traces",
            Some(&admin()),
            &[("content-type", "application/json")],
            b"{}",
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(
        body_text(&answer),
        r#"{"message": "Agent tracing is not enabled. Set `tracing:` in general_settings and CLICKHOUSE_URL."}"#
    );
}

#[rstest]
#[case::too_large(StatusCode::PAYLOAD_TOO_LARGE, "payload too large")]
#[case::invalid(StatusCode::BAD_REQUEST, "ingest invalid")]
#[tokio::test]
async fn ingest_failures_keep_their_status(#[case] status: StatusCode, #[case] message: &str) {
    let mut fake = FakeRuntime::new();
    fake.ingest = if status == StatusCode::PAYLOAD_TOO_LARGE {
        super::support::IngestAnswer::TooLarge(message.to_owned())
    } else {
        super::support::IngestAnswer::Invalid(message.to_owned())
    };
    let runtime = Arc::new(fake);
    let answer = post_traces(&runtime, Some(&admin()), "application/json", b"{}").await;
    assert_eq!(answer.status, status);
    assert_eq!(
        body_text(&answer),
        format!("{{\"message\": \"{message}\"}}")
    );
}

#[tokio::test]
async fn ingest_unavailable_is_503_with_retry_after() {
    let mut fake = FakeRuntime::new();
    fake.ingest = super::support::IngestAnswer::Storage;
    let runtime = Arc::new(fake);
    let answer = post_traces(&runtime, Some(&admin()), "application/json", b"{}").await;
    assert_eq!(answer.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        answer.headers.get(header::RETRY_AFTER),
        Some(&header_value("2"))
    );
    assert_eq!(
        body_text(&answer),
        r#"{"message": "Trace ingestion is temporarily unavailable"}"#
    );
}

#[tokio::test]
async fn list_success_returns_the_page() {
    let mut fake = FakeRuntime::new();
    fake.list = ReadAnswer::Ok(page());
    let runtime = Arc::new(fake);
    let answer = send(
        app(&runtime, None),
        build_request(
            Method::GET,
            "/v1/traces?start_ms=100&end_ms=200&cursor=abc",
            Some(&team_member()),
            &[],
            b"",
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    assert!(body_text(&answer).contains("\"next_cursor\":\"next\""));
    let seen = runtime
        .seen_list
        .lock()
        .expect("lock")
        .clone()
        .expect("seen");
    assert_eq!(seen.start_ms, 100);
    assert_eq!(seen.end_ms, 200);
    assert_eq!(seen.cursor.as_deref(), Some("abc"));
    assert_eq!(seen.scope.team_ids, vec!["team-1".to_owned()]);
}

#[tokio::test]
async fn list_defaults_the_window_to_the_last_day() {
    let runtime = Arc::new(FakeRuntime::new());
    let answer = send(
        app(&runtime, None),
        build_request(Method::GET, "/v1/traces", Some(&admin()), &[], b""),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    let seen = runtime
        .seen_list
        .lock()
        .expect("lock")
        .clone()
        .expect("seen");
    assert_eq!(seen.start_ms, NOW_MS - DAY_MS);
    assert_eq!(seen.end_ms, NOW_MS);
    assert!(seen.scope.team_ids.is_empty());
    assert_eq!(seen.scope.api_key_hash, "");
}

#[tokio::test]
async fn list_token_scope_reaches_the_receiver() {
    let runtime = Arc::new(FakeRuntime::new());
    let caller = principal(vec![Role::TeamMember], None, Some("hash-1"), None, None);
    let answer = send(
        app(&runtime, None),
        build_request(
            Method::GET,
            "/v1/traces?start_ms=1&end_ms=2",
            Some(&caller),
            &[],
            b"",
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    let seen = runtime
        .seen_list
        .lock()
        .expect("lock")
        .clone()
        .expect("seen");
    assert_eq!(seen.scope.team_ids, vec![String::new()]);
    assert_eq!(seen.scope.api_key_hash, "hash-1");
}

#[tokio::test]
async fn list_without_a_read_scope_is_forbidden() {
    let runtime = Arc::new(FakeRuntime::new());
    let caller = principal(vec![Role::TeamMember], None, None, None, None);
    let answer = send(
        app(&runtime, None),
        build_request(Method::GET, "/v1/traces", Some(&caller), &[], b""),
    )
    .await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN);
    assert_eq!(
        body_text(&answer),
        r#"{"detail":"Not allowed to view agent traces"}"#
    );
}

#[tokio::test]
async fn list_without_a_receiver_is_not_implemented() {
    let answer = send(
        application(None, None),
        build_request(Method::GET, "/v1/traces", Some(&admin()), &[], b""),
    )
    .await;
    assert_eq!(answer.status, StatusCode::NOT_IMPLEMENTED);
    assert!(body_text(&answer).contains("Agent tracing is not enabled"));
}

#[tokio::test]
async fn list_with_an_invalid_cursor_is_a_bad_request() {
    let mut fake = FakeRuntime::new();
    fake.list = ReadAnswer::InvalidCursor("Invalid trace cursor".to_owned());
    let runtime = Arc::new(fake);
    let answer = send(
        app(&runtime, None),
        build_request(Method::GET, "/v1/traces", Some(&admin()), &[], b""),
    )
    .await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    assert_eq!(body_text(&answer), r#"{"detail":"Invalid trace cursor"}"#);
}

#[tokio::test]
async fn list_rejects_a_non_integer_window() {
    let runtime = Arc::new(FakeRuntime::new());
    let answer = send(
        app(&runtime, None),
        build_request(
            Method::GET,
            "/v1/traces?start_ms=yesterday",
            Some(&admin()),
            &[],
            b"",
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body_text(&answer),
        r#"{"detail":"invalid trace query parameters"}"#
    );
}

#[tokio::test]
async fn list_storage_failure_is_a_shared_error() {
    let mut fake = FakeRuntime::new();
    fake.list = ReadAnswer::Storage;
    let runtime = Arc::new(fake);
    let answer = send(
        app(&runtime, None),
        build_request(Method::GET, "/v1/traces", Some(&admin()), &[], b""),
    )
    .await;
    assert_eq!(answer.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body_text(&answer),
        r#"{"error":{"message":"Internal server error","type":"internal_server_error","param":null,"code":"500"}}"#
    );
}

#[tokio::test]
async fn get_trace_found_is_200_and_passes_the_ref() {
    let mut fake = FakeRuntime::new();
    fake.trace = ReadAnswer::Ok(Some(trace()));
    let runtime = Arc::new(fake);
    let answer = send(
        app(&runtime, None),
        build_request(
            Method::GET,
            "/v1/traces/trace-1?trace_ref=env-a",
            Some(&admin()),
            &[],
            b"",
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    assert!(body_text(&answer).contains("\"trace_id\":\"trace-1\""));
    let seen = runtime
        .seen_trace
        .lock()
        .expect("lock")
        .clone()
        .expect("seen");
    assert_eq!(seen, ("trace-1".to_owned(), "env-a".to_owned()));
}

#[tokio::test]
async fn get_trace_missing_is_404() {
    let runtime = Arc::new(FakeRuntime::new());
    let answer = send(
        app(&runtime, None),
        build_request(Method::GET, "/v1/traces/trace-9", Some(&admin()), &[], b""),
    )
    .await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND);
    assert_eq!(
        body_text(&answer),
        r#"{"detail":"Trace trace-9 not found"}"#
    );
}

#[tokio::test]
async fn get_span_found_is_200() {
    let mut fake = FakeRuntime::new();
    fake.span = ReadAnswer::Ok(Some(span_detail()));
    let runtime = Arc::new(fake);
    let answer = send(
        app(&runtime, None),
        build_request(
            Method::GET,
            "/v1/traces/trace-1/spans/span-1?trace_ref=env-a",
            Some(&team_member()),
            &[],
            b"",
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    assert!(body_text(&answer).contains("\"span_id\":\"span-1\""));
    let seen = runtime
        .seen_span
        .lock()
        .expect("lock")
        .clone()
        .expect("seen");
    assert_eq!(seen.trace_id, "trace-1");
    assert_eq!(seen.span_id, "span-1");
    assert_eq!(seen.trace_ref, "env-a");
    assert_eq!(seen.scope.team_ids, vec!["team-1".to_owned()]);
    assert_eq!(seen.scope.api_key_hash, "");
}

#[tokio::test]
async fn get_span_missing_is_404() {
    let runtime = Arc::new(FakeRuntime::new());
    let answer = send(
        app(&runtime, None),
        build_request(
            Method::GET,
            "/v1/traces/trace-1/spans/span-9",
            Some(&admin()),
            &[],
            b"",
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND);
    assert_eq!(body_text(&answer), r#"{"detail":"Span span-9 not found"}"#);
}

#[tokio::test]
async fn get_span_error_found_is_200_and_passes_the_cursor() {
    let mut fake = FakeRuntime::new();
    fake.span_error = ReadAnswer::Ok(Some(span_error_page()));
    let runtime = Arc::new(fake);
    let answer = send(
        app(&runtime, None),
        build_request(
            Method::GET,
            "/v1/traces/trace-1/spans/span-1/error?cursor=abc&trace_ref=env-a",
            Some(&team_member()),
            &[],
            b"",
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    assert!(body_text(&answer).contains("\"message\":\"boom\""));
    let seen = runtime
        .seen_error
        .lock()
        .expect("lock")
        .clone()
        .expect("seen");
    assert_eq!(seen.cursor.as_deref(), Some("abc"));
    assert_eq!(seen.trace_ref, "env-a");
    assert_eq!(seen.span_id, "span-1");
    assert_eq!(seen.trace_id, "trace-1");
    assert_eq!(seen.scope.team_ids, vec!["team-1".to_owned()]);
}

#[tokio::test]
async fn get_span_error_missing_is_404() {
    let runtime = Arc::new(FakeRuntime::new());
    let answer = send(
        app(&runtime, None),
        build_request(
            Method::GET,
            "/v1/traces/trace-1/spans/span-1/error",
            Some(&admin()),
            &[],
            b"",
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND);
    assert_eq!(
        body_text(&answer),
        r#"{"detail":"Span diagnostic not found or no longer available"}"#
    );
}

#[tokio::test]
async fn get_span_error_rejects_an_overlong_cursor() {
    let runtime = Arc::new(FakeRuntime::new());
    let cursor = "a".repeat(513);
    let uri = format!("/v1/traces/trace-1/spans/span-1/error?cursor={cursor}");
    let answer = send(
        app(&runtime, None),
        build_request(Method::GET, &uri, Some(&admin()), &[], b""),
    )
    .await;
    assert_eq!(answer.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body_text(&answer).contains("string_too_long"));
}

#[tokio::test]
async fn get_span_error_checks_the_cursor_before_the_read_scope() {
    let runtime = Arc::new(FakeRuntime::new());
    let caller = principal(vec![Role::TeamMember], None, None, None, None);
    let cursor = "a".repeat(513);
    let uri = format!("/v1/traces/trace-1/spans/span-1/error?cursor={cursor}");
    let answer = send(
        app(&runtime, None),
        build_request(Method::GET, &uri, Some(&caller), &[], b""),
    )
    .await;
    assert_eq!(answer.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn get_span_error_accepts_a_cursor_of_exactly_512_characters() {
    let runtime = Arc::new(FakeRuntime::new());
    let cursor = "a".repeat(512);
    let uri = format!("/v1/traces/trace-1/spans/span-1/error?cursor={cursor}");
    let answer = send(
        app(&runtime, None),
        build_request(Method::GET, &uri, Some(&admin()), &[], b""),
    )
    .await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn get_trace_without_a_receiver_is_not_implemented() {
    let answer = send(
        application(None, None),
        build_request(Method::GET, "/v1/traces/trace-1", Some(&admin()), &[], b""),
    )
    .await;
    assert_eq!(answer.status, StatusCode::NOT_IMPLEMENTED);
    assert!(body_text(&answer).contains("Agent tracing is not enabled"));
}

#[tokio::test]
async fn query_without_a_master_key_is_503() {
    let runtime = Arc::new(FakeRuntime::new());
    let answer = send(
        app(&runtime, None),
        build_request(
            Method::POST,
            "/v1/traces/query",
            Some(&admin()),
            &[("content-type", "application/json")],
            br#"{"sql":"SELECT 1"}"#,
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body_text(&answer),
        r#"{"detail":"Trace SQL queries require a configured proxy master key"}"#
    );
}

#[tokio::test]
async fn query_without_a_receiver_but_with_a_secret_is_501() {
    let answer = send(
        application(None, Some("sk-master")),
        build_request(
            Method::POST,
            "/v1/traces/query",
            Some(&admin()),
            &[("content-type", "application/json")],
            br#"{"sql":"SELECT 1"}"#,
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn query_admin_scope_reaches_the_storage() {
    let runtime = Arc::new(FakeRuntime::new());
    let answer = send(
        app(&runtime, Some("sk-master")),
        build_request(
            Method::POST,
            "/v1/traces/query",
            Some(&admin()),
            &[("content-type", "application/json")],
            br#"{"sql":"SELECT 1"}"#,
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(body_text(&answer), "{}");
    assert_eq!(
        answer.headers.get(header::CONTENT_TYPE),
        Some(&header_value("application/json"))
    );
    let seen = runtime.queries.seen.lock().expect("lock").clone();
    let entry = seen.first().expect("seen query");
    assert_eq!(entry.scope, QueryScope::Admin);
    assert_eq!(entry.sql.as_deref(), Some("SELECT 1"));
    assert_eq!(entry.secret, "sk-master");
}

#[tokio::test]
async fn query_team_scope_is_derived_from_the_team() {
    let runtime = Arc::new(FakeRuntime::new());
    let caller = principal(vec![Role::TeamMember], Some("team-1"), None, None, None);
    let answer = send(
        app(&runtime, Some("sk-master")),
        build_request(
            Method::POST,
            "/v1/traces/query",
            Some(&caller),
            &[("content-type", "application/json")],
            br#"{"sql":"SELECT 1"}"#,
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    let seen = runtime.queries.seen.lock().expect("lock").clone();
    assert_eq!(
        seen.first().expect("seen query").scope,
        QueryScope::Team {
            team_id: "team-1".to_owned()
        }
    );
}

#[tokio::test]
async fn query_key_scope_is_derived_from_the_token() {
    let runtime = Arc::new(FakeRuntime::new());
    let caller = principal(vec![Role::TeamMember], None, Some("hash-1"), None, None);
    let answer = send(
        app(&runtime, Some("sk-master")),
        build_request(
            Method::POST,
            "/v1/traces/query",
            Some(&caller),
            &[("content-type", "application/json")],
            br#"{"sql":"SELECT 1"}"#,
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    let seen = runtime.queries.seen.lock().expect("lock").clone();
    assert_eq!(
        seen.first().expect("seen query").scope,
        QueryScope::Key {
            team_id: String::new(),
            api_key_hash: "hash-1".to_owned()
        }
    );
}

#[tokio::test]
async fn query_project_scope_uses_the_project_key() {
    let runtime = Arc::new(FakeRuntime::new());
    let caller = principal(
        vec![Role::TeamMember],
        Some("team-1"),
        Some("hash-1"),
        None,
        Some("project-1"),
    );
    let answer = send(
        app(&runtime, Some("sk-master")),
        build_request(
            Method::POST,
            "/v1/traces/query",
            Some(&caller),
            &[("content-type", "application/json")],
            br#"{"sql":"SELECT 1"}"#,
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    let seen = runtime.queries.seen.lock().expect("lock").clone();
    assert_eq!(
        seen.first().expect("seen query").scope,
        QueryScope::Key {
            team_id: "team-1".to_owned(),
            api_key_hash: "hash-1".to_owned()
        }
    );
}

#[tokio::test]
async fn query_project_without_a_key_is_forbidden() {
    let runtime = Arc::new(FakeRuntime::new());
    let caller = principal(vec![Role::TeamMember], None, None, None, Some("project-1"));
    let answer = send(
        app(&runtime, Some("sk-master")),
        build_request(
            Method::POST,
            "/v1/traces/query",
            Some(&caller),
            &[("content-type", "application/json")],
            br#"{"sql":"SELECT 1"}"#,
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN);
    assert_eq!(
        body_text(&answer),
        r#"{"detail":"Project trace SQL queries require a project key"}"#
    );
}

#[tokio::test]
async fn query_without_a_scope_is_forbidden() {
    let runtime = Arc::new(FakeRuntime::new());
    let caller = principal(vec![Role::TeamMember], None, None, None, None);
    let answer = send(
        app(&runtime, Some("sk-master")),
        build_request(
            Method::POST,
            "/v1/traces/query",
            Some(&caller),
            &[("content-type", "application/json")],
            br#"{"sql":"SELECT 1"}"#,
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN);
    assert_eq!(
        body_text(&answer),
        r#"{"detail":"Trace SQL queries require an authenticated trace scope"}"#
    );
}

#[tokio::test]
async fn query_help_returns_the_storage_result() {
    let runtime = Arc::new(FakeRuntime::new());
    let answer = send(
        app(&runtime, Some("sk-master")),
        build_request(
            Method::GET,
            "/v1/traces/query/help",
            Some(&admin()),
            &[],
            b"",
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(body_text(&answer), "{}");
    let seen = runtime.queries.seen.lock().expect("lock").clone();
    let entry = seen.first().expect("seen query");
    assert_eq!(entry.sql, None);
    assert_eq!(entry.scope, QueryScope::Admin);
    assert_eq!(entry.secret, "sk-master");
}

#[tokio::test]
async fn query_help_runtime_failure_is_503() {
    let queries = Arc::new(super::support::FakeQueryStore::new(
        QueryAnswer::Ok("{}".to_owned()),
        QueryAnswer::Unavailable("reader limits".to_owned()),
    ));
    let mut fake = FakeRuntime::new();
    fake.queries = queries;
    let runtime = Arc::new(fake);
    let answer = send(
        app(&runtime, Some("sk-master")),
        build_request(
            Method::GET,
            "/v1/traces/query/help",
            Some(&admin()),
            &[],
            b"",
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body_text(&answer),
        r#"{"detail":"Trace query help is temporarily unavailable"}"#
    );
}

#[tokio::test]
async fn query_invalid_body_is_422() {
    let runtime = Arc::new(FakeRuntime::new());
    let answer = send(
        app(&runtime, Some("sk-master")),
        build_request(
            Method::POST,
            "/v1/traces/query",
            Some(&admin()),
            &[("content-type", "application/json")],
            b"{}",
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn query_invalid_sql_is_400() {
    let queries = Arc::new(super::support::FakeQueryStore::new(
        QueryAnswer::Invalid("only SELECT is allowed".to_owned()),
        QueryAnswer::Ok("{}".to_owned()),
    ));
    let mut fake = FakeRuntime::new();
    fake.queries = queries;
    let runtime = Arc::new(fake);
    let answer = send(
        app(&runtime, Some("sk-master")),
        build_request(
            Method::POST,
            "/v1/traces/query",
            Some(&admin()),
            &[("content-type", "application/json")],
            br#"{"sql":"DROP"}"#,
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    assert_eq!(body_text(&answer), r#"{"detail":"only SELECT is allowed"}"#);
}

#[tokio::test]
async fn query_runtime_failure_is_503() {
    let queries = Arc::new(super::support::FakeQueryStore::new(
        QueryAnswer::Unavailable("clickhouse down".to_owned()),
        QueryAnswer::Ok("{}".to_owned()),
    ));
    let mut fake = FakeRuntime::new();
    fake.queries = queries;
    let runtime = Arc::new(fake);
    let answer = send(
        app(&runtime, Some("sk-master")),
        build_request(
            Method::POST,
            "/v1/traces/query",
            Some(&admin()),
            &[("content-type", "application/json")],
            br#"{"sql":"SELECT 1"}"#,
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body_text(&answer),
        r#"{"detail":"Trace SQL query failed or exceeded reader limits"}"#
    );
}

#[test]
fn the_master_key_record_is_an_admin() {
    assert_eq!(
        TraceAuth::from_principal(&master_principal()).role,
        TraceRole::Admin
    );
}

#[test]
fn the_retry_constant_is_two_seconds() {
    assert_eq!(OTLP_RETRY_AFTER, "2");
}

#[test]
fn the_secret_is_not_empty() {
    let value = SecretString::from("sk-master".to_owned());
    assert_eq!(value.expose_secret(), "sk-master");
}

fn header_value(value: &str) -> HeaderValue {
    HeaderValue::from_str(value).expect("header value")
}
