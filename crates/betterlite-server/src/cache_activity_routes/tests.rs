#![cfg(test)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::header::ALLOW;
use axum::http::{Method, Request, StatusCode};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use clap::Parser as _;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tower::ServiceExt as _;

use super::date::{parse_date, quoted_value};
use super::model::{
    CacheActivityErrorBucket, CacheActivityGroup, CacheActivityResponse, CacheActivityRows,
    compute_totals,
};
use super::query::{CacheActivityQuery, decode};
use super::routes;
use super::store::{CacheActivityStore, DatabaseCacheActivityStore};
use crate::auth::auth_identity::AuthContext;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

const SPEND_TABLES: &str = r#"
CREATE TABLE "LiteLLM_VerificationToken" (
    token text PRIMARY KEY,
    key_alias text
);
CREATE TABLE "LiteLLM_SpendLogs" (
    request_id text,
    call_type text,
    api_key text,
    model text,
    "startTime" timestamp,
    status text,
    cache_hit text,
    completion_tokens integer,
    metadata jsonb
);
"#;

const GROUPS_SEED: &str = r#"
INSERT INTO "LiteLLM_VerificationToken" (token, key_alias) VALUES ('k1', 'alias-a');
INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, model, "startTime", status, cache_hit, completion_tokens) VALUES
    ('r1', 'completion', 'k1', 'm1', '2024-01-10 00:00:00', 'success', 'True', 100),
    ('r2', 'completion', 'k1', 'm1', '2024-01-10 00:00:00', 'success', 'True', 5),
    ('r3', 'completion', 'k1', 'm1', '2024-01-10 00:00:00', 'success', 'False', 40),
    ('r4', 'completion', 'k1', 'm1', '2024-01-10 00:00:00', 'failure', 'False', 10),
    ('r5', 'completion', 'k1', 'm1', '2024-01-10 00:00:00', 'failure', 'True', 20),
    ('r6', 'completion', 'k1', 'm1', '2024-01-10 00:00:00', 'success', 'False', 30);
"#;

const END_DAY_SEED: &str = r#"
INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, model, "startTime", status, cache_hit, completion_tokens) VALUES
    ('r1', 'completion', 'k1', 'm1', '2024-01-01 00:00:00', 'success', 'False', 1),
    ('r2', 'completion', 'k1', 'm1', '2024-01-31 23:59:59', 'success', 'False', 1),
    ('r3', 'completion', 'k1', 'm1', '2024-02-01 00:00:00', 'success', 'False', 1),
    ('r4', 'completion', 'k1', 'm1', '2023-12-31 23:59:59', 'success', 'False', 1);
"#;

const FILTERS_SEED: &str = r#"
INSERT INTO "LiteLLM_VerificationToken" (token, key_alias) VALUES ('k1', 'alias-a'), ('k2', 'alias-b');
INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, model, "startTime", status, cache_hit, completion_tokens, metadata) VALUES
    ('r1', 'completion', 'k1', 'm1', '2024-01-10 00:00:00', 'success', 'False', 1, NULL),
    ('r2', 'completion', 'k2', 'm1', '2024-01-10 00:00:00', 'success', 'False', 1, NULL),
    ('r3', 'completion', 'k1', 'm2', '2024-01-10 00:00:00', 'success', 'False', 1, NULL),
    ('r4', 'completion', 'orphan', 'm3', '2024-01-10 00:00:00', 'success', 'False', 1, NULL),
    ('f1', 'completion', 'k1', 'm1', '2024-01-10 00:00:00', 'failure', 'False', 0, '{"error_information":{"error_code":"rate_limit","error_class":"RateLimitError"}}'),
    ('f2', 'completion', 'k2', 'm1', '2024-01-10 00:00:00', 'failure', 'False', 0, '{"error_information":{"error_code":"timeout","error_class":"TimeoutError"}}');
"#;

const INFO_ROUTES_SEED: &str = r#"
INSERT INTO "LiteLLM_VerificationToken" (token, key_alias) VALUES ('k1', 'alias-a');
INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, model, "startTime", status, cache_hit, completion_tokens, metadata) VALUES
    ('r1', 'completion', 'k1', 'm1', '2024-01-10 00:00:00', 'success', 'False', 1, NULL),
    ('r2', '/key/info', 'k1', 'm-info', '2024-01-10 00:00:00', 'success', 'False', 1, NULL),
    ('r3', 'completion', 'orphan', '', '2024-01-10 00:00:00', 'success', 'False', 1, NULL),
    ('r4', '/key/info', 'k1', 'm-info', '2024-01-10 00:00:00', 'failure', 'False', 0, '{"error_information":{"error_code":"boom","error_class":"Boom"}}');
"#;

const CALL_TYPE_ORDER_SEED: &str = r#"
INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, model, "startTime", status, cache_hit, completion_tokens) VALUES
    ('a1', 'alpha', 'k1', 'm1', '2024-01-10 00:00:00', 'success', 'False', 1),
    ('a2', 'alpha', 'k1', 'm1', '2024-01-10 00:00:00', 'success', 'False', 1),
    ('a3', 'alpha', 'k1', 'm1', '2024-01-10 00:00:00', 'success', 'False', 1),
    ('u1', '', 'k1', 'm1', '2024-01-10 00:00:00', 'success', 'False', 1),
    ('u2', '', 'k1', 'm1', '2024-01-10 00:00:00', 'success', 'False', 1),
    ('b1', 'beta', 'k1', 'm1', '2024-01-10 00:00:00', 'success', 'False', 1);
"#;

const ERROR_BREAKDOWN_SEED: &str = r#"
INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, model, "startTime", status, cache_hit, completion_tokens, metadata) VALUES
    ('e1', 'completion', 'k1', 'm1', '2024-01-10 00:00:00', 'failure', 'False', 0, '{"error_information":{"error_code":"rate_limit_exceeded","error_class":"RateLimitError"}}'),
    ('e2', 'completion', 'k1', 'm1', '2024-01-10 00:00:00', 'failure', 'False', 0, '{"error_information":{"error_code":"rate_limit_exceeded","error_class":"RateLimitError"}}'),
    ('e3', 'completion', 'k1', 'm1', '2024-01-10 00:00:00', 'failure', 'False', 0, '{"error_information":{"error_code":"","error_class":""}}'),
    ('e4', 'completion', 'k1', 'm1', '2024-01-10 00:00:00', 'failure', 'False', 0, NULL),
    ('e5', 'completion', 'k1', 'm1', '2024-01-10 00:00:00', 'failure', 'False', 0, '{}'),
    ('e6', '', 'k1', 'm1', '2024-01-10 00:00:00', 'failure', 'False', 0, '{"error_information":{"error_code":"bad_request","error_class":"BadRequestError"}}'),
    ('s1', 'completion', 'k1', 'm1', '2024-01-10 00:00:00', 'success', 'False', 1, '{"error_information":{"error_code":"rate_limit_exceeded","error_class":"RateLimitError"}}');
"#;

const SAMPLE_REPORT_JSON: &str = r#"{"groups":[{"call_type":"completion","api_requests":1,"cache_hits":1,"failed_requests":0,"cached_completion_tokens":10,"generated_completion_tokens":0}],"totals":{"api_requests":1,"cache_hits":1,"failed_requests":0,"cached_completion_tokens":10,"cache_hit_ratio":50.0},"filter_options":{"key_aliases":["alias-a"],"models":["m1"]},"error_breakdown":[{"call_type":"completion","error_code":"rate_limit","error_class":"RateLimitError","count":1}]}"#;

const REPORT_URI: &str = "/global/activity/cache_hits?start_date=2024-01-01&end_date=2024-01-31";

fn midnight(year: i32, month: u32, day: u32) -> DateTime<Utc> {
    NaiveDate::from_ymd_opt(year, month, day)
        .expect("calendar date")
        .and_time(NaiveTime::MIN)
        .and_utc()
}

fn sample_rows() -> CacheActivityRows {
    CacheActivityRows {
        groups: vec![CacheActivityGroup {
            call_type: "completion".to_owned(),
            api_requests: 1,
            cache_hits: 1,
            failed_requests: 0,
            cached_completion_tokens: 10,
            generated_completion_tokens: 0,
        }],
        error_breakdown: vec![CacheActivityErrorBucket {
            call_type: "completion".to_owned(),
            error_code: "rate_limit".to_owned(),
            error_class: "RateLimitError".to_owned(),
            count: 1,
        }],
        key_aliases: vec!["alias-a".to_owned()],
        models: vec!["m1".to_owned()],
    }
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["cache-activity-routes-tests"])
        .url
        .map(SecretString::from);
    let fixture =
        PostgresFixture::provision_with_statements(url.as_ref(), vec![SPEND_TABLES.to_owned()])
            .resolve()
            .await
            .expect("postgres fixture");
    let settings = fixture
        .options
        .url
        .expose_secret()
        .parse()
        .expect("postgres url");
    let manager = Manager::new(settings, NoTls);
    let pool = Pool::builder(manager)
        .max_size(1)
        .build()
        .expect("postgres pool");
    (fixture, pool)
}

async fn execute(pool: &Pool, sql: &str) {
    let client = pool.get().await.expect("postgres client");
    client.batch_execute(sql).await.expect("seed sql");
}

fn store(pool: &Pool) -> DatabaseCacheActivityStore {
    DatabaseCacheActivityStore {
        store: Arc::new(PostgresStore {
            pool: Some(pool.clone()),
        }),
    }
}

fn caller() -> AuthContext {
    let principal: Principal = serde_json::from_str(
        r#"{"principal_type":"human","subject":"user","roles":[],"auth_method":"api_key"}"#,
    )
    .expect("test principal");
    AuthContext { principal }
}

fn request(method: Method, uri: &str, caller: Option<&AuthContext>) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .expect("request");
    if let Some(context) = caller {
        request.extensions_mut().insert(context.clone());
    }
    request
}

struct Reply {
    status: StatusCode,
    allow: Option<String>,
    body: String,
}

async fn call(app: Router, request: Request<Body>) -> Reply {
    let response = app.oneshot(request).await.expect("router response");
    let status = response.status();
    let allow = response
        .headers()
        .get(ALLOW)
        .map(|value| value.to_str().expect("ascii header").to_owned());
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    Reply {
        status,
        allow,
        body: String::from_utf8(body.to_vec()).expect("utf-8 body"),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Seen {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    key_aliases: Vec<String>,
    models: Vec<String>,
}

struct FakeStore {
    rows: CacheActivityRows,
    failure: bool,
    seen: Mutex<Option<Seen>>,
}

impl FakeStore {
    fn answering(rows: CacheActivityRows) -> Self {
        Self {
            rows,
            failure: false,
            seen: Mutex::new(None),
        }
    }

    fn failing() -> Self {
        Self {
            rows: CacheActivityRows::default(),
            failure: true,
            seen: Mutex::new(None),
        }
    }

    fn recorded(&self) -> Option<Seen> {
        self.seen.lock().expect("seen lock").clone()
    }
}

#[async_trait]
impl CacheActivityStore for FakeStore {
    async fn read(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        key_aliases: &[String],
        models: &[String],
    ) -> Result<CacheActivityRows, DbError> {
        *self.seen.lock().expect("seen lock") = Some(Seen {
            start,
            end,
            key_aliases: key_aliases.to_vec(),
            models: models.to_vec(),
        });
        if self.failure {
            return Err(DbError::NoDbConnected);
        }
        Ok(self.rows.clone())
    }
}

fn fake_store(handle: &Arc<FakeStore>) -> Option<Arc<dyn CacheActivityStore>> {
    let shared: Arc<dyn CacheActivityStore> = Arc::clone(handle);
    Some(shared)
}

#[tokio::test]
async fn route_requires_an_authenticated_caller() {
    let reply = call(routes(None), request(Method::GET, REPORT_URI, None)).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        reply.body,
        r#"{"error":{"message":"the API key is not valid","type":"auth_error","param":"None","code":"401"}}"#
    );
}

#[tokio::test]
async fn route_a_missing_start_date_answers_422_before_the_database_check() {
    let caller = caller();
    let reply = call(
        routes(None),
        request(
            Method::GET,
            "/global/activity/cache_hits?end_date=2024-01-31",
            Some(&caller),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.body,
        r#"{"detail":[{"type":"missing","loc":["query","start_date"],"msg":"Field required","input":null}]}"#
    );
}

#[tokio::test]
async fn route_a_missing_end_date_answers_422() {
    let caller = caller();
    let store = Arc::new(FakeStore::answering(CacheActivityRows::default()));
    let reply = call(
        routes(fake_store(&store)),
        request(
            Method::GET,
            "/global/activity/cache_hits?start_date=2024-01-01",
            Some(&caller),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.body,
        r#"{"detail":[{"type":"missing","loc":["query","end_date"],"msg":"Field required","input":null}]}"#
    );
}

#[tokio::test]
async fn route_two_missing_dates_are_listed_start_first() {
    let caller = caller();
    let reply = call(
        routes(None),
        request(
            Method::GET,
            "/global/activity/cache_hits?models=m1",
            Some(&caller),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.body,
        r#"{"detail":[{"type":"missing","loc":["query","start_date"],"msg":"Field required","input":null},{"type":"missing","loc":["query","end_date"],"msg":"Field required","input":null}]}"#
    );
}

#[tokio::test]
async fn route_missing_database_answers_500_before_parsing_the_dates() {
    let caller = caller();
    let reply = call(
        routes(None),
        request(
            Method::GET,
            "/global/activity/cache_hits?start_date=2024-13-01&end_date=2024-01-01",
            Some(&caller),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.body,
        r#"{"detail":{"error":"Database not connected. Connect a database to your proxy - https://docs.litellm.ai/docs/simple_proxy#managing-auth---virtual-keys"}}"#
    );
}

#[tokio::test]
async fn route_a_bad_start_date_answers_400_with_python_repr() {
    let caller = caller();
    let store = Arc::new(FakeStore::answering(CacheActivityRows::default()));
    let reply = call(
        routes(fake_store(&store)),
        request(
            Method::GET,
            "/global/activity/cache_hits?start_date=2024-13-01&end_date=2024-01-31",
            Some(&caller),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.body,
        r#"{"detail":{"error":"start_date must be a YYYY-MM-DD date, got '2024-13-01'"}}"#
    );
    assert_eq!(store.recorded(), None);
}

#[tokio::test]
async fn route_a_bad_end_date_answers_400() {
    let caller = caller();
    let store = Arc::new(FakeStore::answering(CacheActivityRows::default()));
    let reply = call(
        routes(fake_store(&store)),
        request(
            Method::GET,
            "/global/activity/cache_hits?start_date=2024-01-01&end_date=2024-01-32",
            Some(&caller),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.body,
        r#"{"detail":{"error":"end_date must be a YYYY-MM-DD date, got '2024-01-32'"}}"#
    );
}

#[tokio::test]
async fn route_the_start_date_is_parsed_before_the_end_date() {
    let caller = caller();
    let store = Arc::new(FakeStore::answering(CacheActivityRows::default()));
    let reply = call(
        routes(fake_store(&store)),
        request(
            Method::GET,
            "/global/activity/cache_hits?start_date=2024-13-01&end_date=2024-01-32",
            Some(&caller),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.body,
        r#"{"detail":{"error":"start_date must be a YYYY-MM-DD date, got '2024-13-01'"}}"#
    );
}

#[tokio::test]
async fn route_a_single_quote_date_uses_double_quoted_python_repr() {
    let caller = caller();
    let store = Arc::new(FakeStore::answering(CacheActivityRows::default()));
    let reply = call(
        routes(fake_store(&store)),
        request(
            Method::GET,
            "/global/activity/cache_hits?start_date=2024-01-0%275&end_date=2024-01-31",
            Some(&caller),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.body,
        r#"{"detail":{"error":"start_date must be a YYYY-MM-DD date, got \"2024-01-0'5\""}}"#
    );
}

#[tokio::test]
async fn route_store_failures_reach_the_generic_internal_error() {
    let caller = caller();
    let store = Arc::new(FakeStore::failing());
    let reply = call(
        routes(fake_store(&store)),
        request(Method::GET, REPORT_URI, Some(&caller)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.body,
        r#"{"error":{"message":"Internal server error","type":"internal_server_error","param":null,"code":"500"}}"#
    );
}

#[tokio::test]
async fn route_filters_and_dates_reach_the_store() {
    let caller = caller();
    let store = Arc::new(FakeStore::answering(CacheActivityRows::default()));
    let reply = call(
        routes(fake_store(&store)),
        request(
            Method::GET,
            "/global/activity/cache_hits?start_date=2023-12-31&end_date=2024-1-5&key_aliases=a&key_aliases=b&models=m&models=",
            Some(&caller),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.recorded(),
        Some(Seen {
            start: midnight(2023, 12, 31),
            end: midnight(2024, 1, 5),
            key_aliases: vec!["a".to_owned(), "b".to_owned()],
            models: vec!["m".to_owned(), String::new()],
        })
    );
}

#[tokio::test]
async fn route_answers_the_cache_activity_report() {
    let caller = caller();
    let store = Arc::new(FakeStore::answering(sample_rows()));
    let reply = call(
        routes(fake_store(&store)),
        request(Method::GET, REPORT_URI, Some(&caller)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, SAMPLE_REPORT_JSON);
}

#[tokio::test]
async fn route_post_answers_method_not_allowed() {
    let caller = caller();
    let reply = call(
        routes(None),
        request(Method::POST, REPORT_URI, Some(&caller)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(reply.allow.as_deref(), Some("GET"));
    assert_eq!(reply.body, r#"{"detail":"Method Not Allowed"}"#);
}

#[test]
fn date_accepts_zero_padded_and_unpadded_components() {
    assert_eq!(parse_date("2024-01-05"), Some(midnight(2024, 1, 5)));
    assert_eq!(parse_date("2024-1-5"), Some(midnight(2024, 1, 5)));
    assert_eq!(parse_date("2024-01-5"), Some(midnight(2024, 1, 5)));
    assert_eq!(parse_date("2024-1-05"), Some(midnight(2024, 1, 5)));
}

#[test]
fn date_accepts_a_space_before_a_one_digit_day() {
    assert_eq!(parse_date("2024-01- 5"), Some(midnight(2024, 1, 5)));
    assert_eq!(parse_date("2024-1- 9"), Some(midnight(2024, 1, 9)));
}

#[test]
fn date_accepts_a_leap_day_and_the_year_bounds() {
    assert_eq!(parse_date("2024-02-29"), Some(midnight(2024, 2, 29)));
    assert_eq!(parse_date("0001-01-01"), Some(midnight(1, 1, 1)));
    assert_eq!(parse_date("9999-12-31"), Some(midnight(9999, 12, 31)));
}

#[test]
fn date_rejects_an_impossible_day() {
    assert_eq!(parse_date("2024-02-30"), None);
    assert_eq!(parse_date("2023-02-29"), None);
    assert_eq!(parse_date("2024-04-31"), None);
    assert_eq!(parse_date("2024-01-32"), None);
}

#[test]
fn date_rejects_a_space_before_a_two_digit_day() {
    assert_eq!(parse_date("2024-01- 05"), None);
    assert_eq!(parse_date("2024-01- 0"), None);
}

#[test]
fn date_rejects_trailing_text_and_surrounding_whitespace() {
    assert_eq!(parse_date("2024-01-05x"), None);
    assert_eq!(parse_date("2024-01-05 "), None);
    assert_eq!(parse_date(" 2024-01-05"), None);
    assert_eq!(parse_date("2024-01-05\n"), None);
}

#[test]
fn date_rejects_non_ascii_digits() {
    assert_eq!(parse_date("\u{0662}\u{0660}\u{0662}\u{0664}-01-05"), None);
    assert_eq!(parse_date("2024-\u{0660}\u{0661}-05"), None);
}

#[test]
fn date_rejects_year_zero() {
    assert_eq!(parse_date("0000-01-01"), None);
}

#[test]
fn date_rejects_zero_month_and_day() {
    assert_eq!(parse_date("2024-00-01"), None);
    assert_eq!(parse_date("2024-01-00"), None);
}

#[test]
fn date_repr_quotes_with_single_quotes() {
    assert_eq!(quoted_value("2024-13-01"), "'2024-13-01'");
    assert_eq!(quoted_value(""), "''");
}

#[test]
fn date_repr_switches_to_double_quotes_for_a_single_quote() {
    assert_eq!(quoted_value("2024-01-0'5"), "\"2024-01-0'5\"");
}

#[test]
fn date_repr_escapes_backslashes_and_control_characters() {
    assert_eq!(quoted_value("a\\b\n\t\r\u{1}"), r"'a\\b\n\t\r\x01'");
}

#[test]
fn date_repr_escapes_a_single_quote_when_both_quotes_appear() {
    assert_eq!(quoted_value("a'b\"c"), r#"'a\'b"c'"#);
}

#[test]
fn totals_sum_groups_and_report_the_percentage() {
    let totals = compute_totals(&[
        CacheActivityGroup {
            call_type: "a".to_owned(),
            api_requests: 1,
            cache_hits: 1,
            failed_requests: 0,
            cached_completion_tokens: 10,
            generated_completion_tokens: 2,
        },
        CacheActivityGroup {
            call_type: "b".to_owned(),
            api_requests: 2,
            cache_hits: 0,
            failed_requests: 1,
            cached_completion_tokens: 5,
            generated_completion_tokens: 4,
        },
    ]);
    assert_eq!(totals.api_requests, 3);
    assert_eq!(totals.cache_hits, 1);
    assert_eq!(totals.failed_requests, 1);
    assert_eq!(totals.cached_completion_tokens, 15);
    assert!(totals.cache_hit_ratio.total_cmp(&20.0_f64).is_eq());
}

#[test]
fn totals_ratio_is_zero_without_requests() {
    let totals = compute_totals(&[]);
    assert_eq!(totals.api_requests, 0);
    assert_eq!(totals.cache_hits, 0);
    assert_eq!(totals.failed_requests, 0);
    assert_eq!(totals.cached_completion_tokens, 0);
    assert!(totals.cache_hit_ratio.total_cmp(&0.0_f64).is_eq());
}

#[test]
fn totals_zero_request_groups_keep_their_tokens_and_a_zero_ratio() {
    let totals = compute_totals(&[CacheActivityGroup {
        call_type: "a".to_owned(),
        api_requests: 0,
        cache_hits: 0,
        failed_requests: 0,
        cached_completion_tokens: 3,
        generated_completion_tokens: 0,
    }]);
    assert_eq!(totals.cached_completion_tokens, 3);
    assert!(totals.cache_hit_ratio.total_cmp(&0.0_f64).is_eq());
}

#[test]
fn totals_response_serializes_counts_as_integers_in_litellm_order() {
    let response = CacheActivityResponse::new(sample_rows());
    assert_eq!(
        serde_json::to_string(&response).expect("response json"),
        SAMPLE_REPORT_JSON
    );
}

#[test]
fn totals_empty_report_serializes_zero_counts_and_zero_ratio() {
    let response = CacheActivityResponse::new(CacheActivityRows::default());
    assert_eq!(
        serde_json::to_string(&response).expect("response json"),
        r#"{"groups":[],"totals":{"api_requests":0,"cache_hits":0,"failed_requests":0,"cached_completion_tokens":0,"cache_hit_ratio":0.0},"filter_options":{"key_aliases":[],"models":[]},"error_breakdown":[]}"#
    );
}

#[test]
fn query_a_missing_date_is_reported() {
    assert_eq!(decode(Some("end_date=2024-01-31")), Err(vec!["start_date"]));
    assert_eq!(decode(Some("start_date=2024-01-01")), Err(vec!["end_date"]));
    assert_eq!(
        decode(Some("models=m1")),
        Err(vec!["start_date", "end_date"])
    );
}

#[test]
fn query_keeps_the_last_value_of_a_repeated_date() {
    let query = decode(Some(
        "start_date=2024-01-01&start_date=2024-02-01&end_date=2024-03-01&end_date=2024-04-01",
    ))
    .unwrap();
    assert_eq!(query.start_date, "2024-02-01");
    assert_eq!(query.end_date, "2024-04-01");
}

#[test]
fn query_keeps_repeated_lists_in_order_and_an_empty_value_as_empty() {
    let query = decode(Some(
        "start_date=2024-01-01&end_date=2024-01-31&key_aliases=b&key_aliases=a&models=m1&models=",
    ))
    .unwrap();
    assert_eq!(query.key_aliases, vec!["b".to_owned(), "a".to_owned()]);
    assert_eq!(query.models, vec!["m1".to_owned(), String::new()]);
}

#[test]
fn query_without_lists_keeps_empty_lists() {
    let query: CacheActivityQuery =
        decode(Some("start_date=2024-01-01&end_date=2024-01-31")).unwrap();
    assert_eq!(query.key_aliases, Vec::<String>::new());
    assert_eq!(query.models, Vec::<String>::new());
}

#[tokio::test]
async fn query_groups_split_cache_hits_failures_and_other_requests() {
    let (fixture, pool) = database().await;
    execute(&pool, GROUPS_SEED).await;
    let rows = store(&pool)
        .read(midnight(2024, 1, 1), midnight(2024, 1, 31), &[], &[])
        .await
        .expect("activity rows");
    assert_eq!(rows.groups.len(), 1);
    let group = &rows.groups[0];
    assert_eq!(group.call_type, "completion");
    assert_eq!(group.api_requests, 1);
    assert_eq!(group.cache_hits, 3);
    assert_eq!(group.failed_requests, 2);
    assert_eq!(group.cached_completion_tokens, 125);
    assert_eq!(group.generated_completion_tokens, 80);
    pool.close();
    fixture.close_async().await.expect("fixture close");
}

#[tokio::test]
async fn query_end_date_includes_the_whole_end_day() {
    let (fixture, pool) = database().await;
    execute(&pool, END_DAY_SEED).await;
    let rows = store(&pool)
        .read(midnight(2024, 1, 1), midnight(2024, 1, 31), &[], &[])
        .await
        .expect("activity rows");
    assert_eq!(rows.groups.len(), 1);
    assert_eq!(rows.groups[0].api_requests, 2);
    assert_eq!(rows.groups[0].generated_completion_tokens, 2);
    pool.close();
    fixture.close_async().await.expect("fixture close");
}

#[tokio::test]
async fn query_filters_apply_to_groups_and_errors_but_not_filter_options() {
    let (fixture, pool) = database().await;
    execute(&pool, FILTERS_SEED).await;
    let rows = store(&pool)
        .read(
            midnight(2024, 1, 1),
            midnight(2024, 1, 31),
            &["alias-a".to_owned()],
            &["m1".to_owned()],
        )
        .await
        .expect("activity rows");
    assert_eq!(rows.groups.len(), 1);
    assert_eq!(rows.groups[0].api_requests, 1);
    assert_eq!(rows.error_breakdown.len(), 1);
    assert_eq!(rows.error_breakdown[0].error_code, "rate_limit");
    let mut aliases = rows.key_aliases;
    aliases.sort();
    assert_eq!(
        aliases,
        vec![
            "Unnamed Key".to_owned(),
            "alias-a".to_owned(),
            "alias-b".to_owned()
        ]
    );
    let mut models = rows.models;
    models.sort();
    assert_eq!(
        models,
        vec!["m1".to_owned(), "m2".to_owned(), "m3".to_owned()]
    );
    pool.close();
    fixture.close_async().await.expect("fixture close");
}

#[tokio::test]
async fn query_info_route_call_types_are_excluded_and_empty_models_are_skipped() {
    let (fixture, pool) = database().await;
    execute(&pool, INFO_ROUTES_SEED).await;
    let rows = store(&pool)
        .read(midnight(2024, 1, 1), midnight(2024, 1, 31), &[], &[])
        .await
        .expect("activity rows");
    assert_eq!(rows.groups.len(), 1);
    assert_eq!(rows.groups[0].call_type, "completion");
    assert_eq!(rows.groups[0].api_requests, 2);
    assert_eq!(rows.error_breakdown.len(), 0);
    let mut aliases = rows.key_aliases;
    aliases.sort();
    assert_eq!(
        aliases,
        vec!["Unnamed Key".to_owned(), "alias-a".to_owned()]
    );
    assert_eq!(rows.models, vec!["m1".to_owned()]);
    pool.close();
    fixture.close_async().await.expect("fixture close");
}

#[tokio::test]
async fn query_groups_merge_empty_call_type_into_unknown_and_order_by_count() {
    let (fixture, pool) = database().await;
    execute(&pool, CALL_TYPE_ORDER_SEED).await;
    let rows = store(&pool)
        .read(midnight(2024, 1, 1), midnight(2024, 1, 31), &[], &[])
        .await
        .expect("activity rows");
    assert_eq!(rows.groups.len(), 3);
    assert_eq!(rows.groups[0].call_type, "alpha");
    assert_eq!(rows.groups[0].api_requests, 3);
    assert_eq!(rows.groups[1].call_type, "Unknown");
    assert_eq!(rows.groups[1].api_requests, 2);
    assert_eq!(rows.groups[2].call_type, "beta");
    assert_eq!(rows.groups[2].api_requests, 1);
    pool.close();
    fixture.close_async().await.expect("fixture close");
}

#[tokio::test]
async fn query_error_breakdown_counts_failures_with_unknown_for_missing_information() {
    let (fixture, pool) = database().await;
    execute(&pool, ERROR_BREAKDOWN_SEED).await;
    let rows = store(&pool)
        .read(midnight(2024, 1, 1), midnight(2024, 1, 31), &[], &[])
        .await
        .expect("activity rows");
    assert_eq!(rows.error_breakdown.len(), 3);
    assert_eq!(rows.error_breakdown[0].call_type, "completion");
    assert_eq!(rows.error_breakdown[0].error_code, "Unknown");
    assert_eq!(rows.error_breakdown[0].error_class, "Unknown");
    assert_eq!(rows.error_breakdown[0].count, 3);
    assert_eq!(rows.error_breakdown[1].error_code, "rate_limit_exceeded");
    assert_eq!(rows.error_breakdown[1].error_class, "RateLimitError");
    assert_eq!(rows.error_breakdown[1].count, 2);
    assert_eq!(rows.error_breakdown[2].call_type, "Unknown");
    assert_eq!(rows.error_breakdown[2].error_code, "bad_request");
    assert_eq!(rows.error_breakdown[2].count, 1);
    pool.close();
    fixture.close_async().await.expect("fixture close");
}
