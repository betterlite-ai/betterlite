#![cfg(test)]

mod queries;
mod support;

use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_db::gateway_request_tracking::GatewayRequestClock;
use chrono::DateTime;
use chrono::Utc;
use rstest::rstest;

use super::EXCEPTIONS_PATH;
use super::METRICS_PATH;
use super::SLOW_RESPONSES_PATH;
use super::STREAMING_METRICS_PATH;
use super::query::MetricsQuery;
use super::query::normalized;
use super::store::ExceptionRow;
use super::store::ExceptionWindow;
use super::store::SeriesValue;
use super::store::SlowRequest;
use super::store::SlowRow;
use super::store::SpendFilters;
use super::store::StreamingWindow;
use support::Call;
use support::DEFAULT_THRESHOLD;
use support::FakeStore;
use support::Fault;
use support::app;
use support::caller;
use support::clock;
use support::days_before;
use support::fixed_now;
use support::get;
use support::instant;
use support::named_latency_series;

const UNAUTHORIZED_BODY: &str = r#"{"error":{"message":"the API key is not valid","type":"auth_error","param":"None","code":"401"}}"#;

const DB_NOT_CONNECTED_BODY: &str = r#"{"error":{"message":"DB not connected. This endpoint needs a database; set DATABASE_URL to a PostgreSQL connection string (postgresql://...) to enable it. See https://docs.litellm.ai/docs/proxy/virtual_keys","type":"internal_error","param":"None","code":"500"}}"#;

const PRISMA_BODY: &str = r#"{"error":{"message":"Prisma Client is not initialized","type":"internal_error","param":"None","code":"500"}}"#;

const INTERNAL_BODY: &str =
    r#"{"error":{"message":"Internal server error","type":"internal_server_error"}}"#;

const NO_DB_CONNECTION_BODY: &str = r#"{"error":{"message":"the model metrics database is unavailable","type":"no_db_connection","param":"None","code":"503"}}"#;

const EMPTY_SERIES: &str = r#"{"data":[],"all_api_bases":[]}"#;

const EMPTY_EXCEPTIONS: &str = r#"{"data":[],"exception_types":[]}"#;

#[rstest]
#[case::streaming(STREAMING_METRICS_PATH)]
#[case::latency(METRICS_PATH)]
#[case::slow(SLOW_RESPONSES_PATH)]
#[case::exceptions(EXCEPTIONS_PATH)]
#[tokio::test]
async fn oneshot_every_route_without_a_key_is_401(#[case] path: &str) {
    let reply = get(app(None, DEFAULT_THRESHOLD), None, path).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{path}");
    assert_eq!(reply.body, UNAUTHORIZED_BODY, "{path}");
}

#[tokio::test]
async fn oneshot_streaming_metrics_without_a_database_uses_the_db_not_connected_message() {
    let reply = get(
        app(None, DEFAULT_THRESHOLD),
        Some(&caller()),
        STREAMING_METRICS_PATH,
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.body, DB_NOT_CONNECTED_BODY);
}

#[tokio::test]
async fn oneshot_model_metrics_without_a_database_uses_the_prisma_message() {
    let reply = get(app(None, DEFAULT_THRESHOLD), Some(&caller()), METRICS_PATH).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.body, PRISMA_BODY);
}

#[tokio::test]
async fn oneshot_slow_responses_without_a_database_uses_the_prisma_message() {
    let reply = get(
        app(None, DEFAULT_THRESHOLD),
        Some(&caller()),
        SLOW_RESPONSES_PATH,
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.body, PRISMA_BODY);
}

#[tokio::test]
async fn oneshot_exceptions_without_a_database_uses_the_prisma_message() {
    let reply = get(
        app(None, DEFAULT_THRESHOLD),
        Some(&caller()),
        EXCEPTIONS_PATH,
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.body, PRISMA_BODY);
}

#[tokio::test]
async fn oneshot_a_bad_datetime_is_reported_before_a_missing_database() {
    let caller = caller();
    let metrics = get(
        app(None, DEFAULT_THRESHOLD),
        Some(&caller),
        "/model/metrics?startTime=yesterday",
    )
    .await;
    assert_eq!(metrics.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        metrics.body,
        r#"{"detail":{"error":"startTime must be an ISO 8601 datetime, got \"yesterday\""}}"#
    );
    let streaming = get(
        app(None, DEFAULT_THRESHOLD),
        Some(&caller),
        "/model/streaming_metrics?startTime=yesterday",
    )
    .await;
    assert_eq!(streaming.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        streaming.body,
        r#"{"detail":{"error":"startTime must be an ISO 8601 datetime, got \"yesterday\""}}"#
    );
}

#[tokio::test]
async fn oneshot_streaming_metrics_defaults_to_the_last_week_without_a_model_group() {
    let store = Arc::new(FakeStore::default());
    let reply = get(
        app(Some(Arc::<FakeStore>::clone(&store)), DEFAULT_THRESHOLD),
        Some(&caller()),
        STREAMING_METRICS_PATH,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, EMPTY_SERIES);
    assert_eq!(
        store.calls(),
        vec![Call::Daily(StreamingWindow {
            model_group: None,
            start: days_before(fixed_now(), 7),
            end: fixed_now(),
        })]
    );
}

#[tokio::test]
async fn oneshot_streaming_metrics_a_single_day_lists_each_request() {
    let mut fake = FakeStore::default();
    fake.rows.same_day = vec![
        SeriesValue {
            key: "req-b".to_owned(),
            api_base: Some("https://api.example.com/v1/openai/".to_owned()),
            model: "gpt-4".to_owned(),
            value: 0.25,
        },
        SeriesValue {
            key: "req-a".to_owned(),
            api_base: None,
            model: "gpt-4".to_owned(),
            value: 0.5,
        },
    ];
    let store = Arc::new(fake);
    let reply = get(
        app(Some(Arc::<FakeStore>::clone(&store)), DEFAULT_THRESHOLD),
        Some(&caller()),
        "/model/streaming_metrics?startTime=2026-03-15T00:00:00&endTime=2026-03-15T23:59:59&_selected_model_group=prod",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.body,
        r#"{"data":[{"date":"req-a","gpt-4":0.5},{"date":"req-b","https://api.example.com/v1":0.25}],"all_api_bases":["gpt-4","https://api.example.com/v1"]}"#
    );
    assert_eq!(
        store.calls(),
        vec![Call::SameDay(StreamingWindow {
            model_group: Some("prod".to_owned()),
            start: instant("2026-03-15 00:00:00"),
            end: instant("2026-03-15 23:59:59"),
        })]
    );
}

#[tokio::test]
async fn oneshot_streaming_metrics_several_days_averages_per_day() {
    let mut fake = FakeStore::default();
    fake.rows.daily = vec![
        SeriesValue {
            key: "2026-03-02".to_owned(),
            api_base: None,
            model: "gpt-4".to_owned(),
            value: 0.125,
        },
        SeriesValue {
            key: "2026-03-01".to_owned(),
            api_base: Some("https://api.example.com/v1".to_owned()),
            model: "gpt-4".to_owned(),
            value: 0.5,
        },
    ];
    let store = Arc::new(fake);
    let reply = get(
        app(Some(Arc::<FakeStore>::clone(&store)), DEFAULT_THRESHOLD),
        Some(&caller()),
        "/model/streaming_metrics?startTime=2026-03-01&endTime=2026-03-10",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.body,
        r#"{"data":[{"date":"2026-03-01","https://api.example.com/v1":0.5},{"date":"2026-03-02","gpt-4":0.125}],"all_api_bases":["gpt-4","https://api.example.com/v1"]}"#
    );
    assert_eq!(
        store.calls(),
        vec![Call::Daily(StreamingWindow {
            model_group: None,
            start: instant("2026-03-01 00:00:00"),
            end: instant("2026-03-10 00:00:00"),
        })]
    );
}

#[tokio::test]
async fn oneshot_streaming_metrics_takes_the_end_from_the_clock_when_only_the_start_is_given() {
    let store = Arc::new(FakeStore::default());
    let reply = get(
        app(Some(Arc::<FakeStore>::clone(&store)), DEFAULT_THRESHOLD),
        Some(&caller()),
        "/model/streaming_metrics?startTime=2026-03-01",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.calls(),
        vec![Call::Daily(StreamingWindow {
            model_group: None,
            start: instant("2026-03-01 00:00:00"),
            end: fixed_now(),
        })]
    );
}

#[tokio::test]
async fn oneshot_streaming_metrics_takes_the_start_from_the_clock_when_only_the_end_is_given() {
    let store = Arc::new(FakeStore::default());
    let reply = get(
        app(Some(Arc::<FakeStore>::clone(&store)), DEFAULT_THRESHOLD),
        Some(&caller()),
        "/model/streaming_metrics?endTime=2026-03-10",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.calls(),
        vec![Call::Daily(StreamingWindow {
            model_group: None,
            start: days_before(fixed_now(), 7),
            end: instant("2026-03-10 00:00:00"),
        })]
    );
}

#[tokio::test]
async fn oneshot_model_metrics_defaults_to_the_configured_group_and_month() {
    let store = Arc::new(FakeStore::default());
    let reply = get(
        app(Some(Arc::<FakeStore>::clone(&store)), DEFAULT_THRESHOLD),
        Some(&caller()),
        METRICS_PATH,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, EMPTY_SERIES);
    assert_eq!(
        store.calls(),
        vec![Call::Latency(SpendFilters {
            model_group: "gpt-4-32k".to_owned(),
            start: days_before(fixed_now(), 28),
            end: fixed_now(),
            api_key: "null".to_owned(),
            customer: "null".to_owned(),
        })]
    );
}

#[tokio::test]
async fn oneshot_model_metrics_sends_the_filters_and_names_bases() {
    let mut fake = FakeStore::default();
    fake.rows.latency = named_latency_series();
    let store = Arc::new(fake);
    let reply = get(
        app(Some(Arc::<FakeStore>::clone(&store)), DEFAULT_THRESHOLD),
        Some(&caller()),
        "/model/metrics?_selected_model_group=chat&api_key=sk-1&customer=alice",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.body,
        r#"{"data":[{"date":"2026-03-10","claude":0.004,"https://api.example.com":0.002},{"date":"2026-03-11","claude":0.008}],"all_api_bases":["claude","https://api.example.com"]}"#
    );
    assert_eq!(
        store.calls(),
        vec![Call::Latency(SpendFilters {
            model_group: "chat".to_owned(),
            start: days_before(fixed_now(), 28),
            end: fixed_now(),
            api_key: "sk-1".to_owned(),
            customer: "alice".to_owned(),
        })]
    );
}

#[tokio::test]
async fn oneshot_model_metrics_treats_undefined_as_no_filter() {
    let store = Arc::new(FakeStore::default());
    let reply = get(
        app(Some(Arc::<FakeStore>::clone(&store)), DEFAULT_THRESHOLD),
        Some(&caller()),
        "/model/metrics?api_key=undefined&customer=undefined",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.calls(),
        vec![Call::Latency(SpendFilters {
            model_group: "gpt-4-32k".to_owned(),
            start: days_before(fixed_now(), 28),
            end: fixed_now(),
            api_key: "null".to_owned(),
            customer: "null".to_owned(),
        })]
    );
}

#[rstest]
#[case::rounds_down(300.75_f64, 300_i64)]
#[case::cuts_the_fraction(12.9_f64, 12_i64)]
#[case::truncates_toward_zero(-1.5_f64, -1_i64)]
#[case::keeps_zero(0.0_f64, 0_i64)]
#[tokio::test]
async fn oneshot_slow_responses_truncates_the_threshold(
    #[case] configured: f64,
    #[case] expected: i64,
) {
    let store = Arc::new(FakeStore::default());
    let reply = get(
        app(Some(Arc::<FakeStore>::clone(&store)), configured),
        Some(&caller()),
        SLOW_RESPONSES_PATH,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{configured}");
    assert_eq!(reply.body, "[]", "{configured}");
    assert_eq!(
        store.calls(),
        vec![Call::Slow(SlowRequest {
            filters: SpendFilters {
                model_group: "gpt-4-32k".to_owned(),
                start: days_before(fixed_now(), 28),
                end: fixed_now(),
                api_key: "null".to_owned(),
                customer: "null".to_owned(),
            },
            threshold_seconds: expected,
        })],
        "{configured}"
    );
}

#[rstest]
#[case::nan(f64::NAN)]
#[case::infinity(f64::INFINITY)]
#[tokio::test]
async fn oneshot_slow_responses_a_threshold_that_is_not_whole_is_500(#[case] configured: f64) {
    let store = Arc::new(FakeStore::default());
    let reply = get(
        app(Some(Arc::<FakeStore>::clone(&store)), configured),
        Some(&caller()),
        SLOW_RESPONSES_PATH,
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.body, INTERNAL_BODY);
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn oneshot_slow_responses_sends_the_filters_and_names_bases() {
    let mut fake = FakeStore::default();
    fake.rows.slow = vec![
        SlowRow {
            api_base: Some("https://api.example.com/openai/v1".to_owned()),
            total_count: 5,
            slow_count: 3,
        },
        SlowRow {
            api_base: None,
            total_count: 2,
            slow_count: 0,
        },
    ];
    let store = Arc::new(fake);
    let reply = get(
        app(Some(Arc::<FakeStore>::clone(&store)), DEFAULT_THRESHOLD),
        Some(&caller()),
        "/model/metrics/slow_responses?_selected_model_group=chat&api_key=sk-1&customer=alice",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.body,
        r#"[{"api_base":"https://api.example.com","total_count":5,"slow_count":3},{"api_base":"","total_count":2,"slow_count":0}]"#
    );
    assert_eq!(
        store.calls(),
        vec![Call::Slow(SlowRequest {
            filters: SpendFilters {
                model_group: "chat".to_owned(),
                start: days_before(fixed_now(), 28),
                end: fixed_now(),
                api_key: "sk-1".to_owned(),
                customer: "alice".to_owned(),
            },
            threshold_seconds: 300,
        })]
    );
}

#[tokio::test]
async fn oneshot_exceptions_pivots_counts_by_type() {
    let mut fake = FakeStore::default();
    fake.rows.exceptions = vec![
        ExceptionRow {
            model: "gpt-4-https://a".to_owned(),
            exception_type: "RateLimitError".to_owned(),
            count: 3,
        },
        ExceptionRow {
            model: "gpt-4-https://a".to_owned(),
            exception_type: "AuthenticationError".to_owned(),
            count: 2,
        },
        ExceptionRow {
            model: "claude".to_owned(),
            exception_type: "RateLimitError".to_owned(),
            count: 1,
        },
    ];
    let store = Arc::new(fake);
    let reply = get(
        app(Some(Arc::<FakeStore>::clone(&store)), DEFAULT_THRESHOLD),
        Some(&caller()),
        EXCEPTIONS_PATH,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.body,
        r#"{"data":[{"model":"gpt-4-https://a","total_exceptions":2,"AuthenticationError":2,"RateLimitError":3},{"model":"claude","total_exceptions":1,"RateLimitError":1}],"exception_types":["AuthenticationError","RateLimitError"]}"#
    );
    assert_eq!(
        store.calls(),
        vec![Call::Exceptions(ExceptionWindow {
            model_group: None,
            start: days_before(fixed_now(), 28),
            end: fixed_now(),
        })]
    );
}

#[tokio::test]
async fn oneshot_exceptions_sends_the_model_group() {
    let store = Arc::new(FakeStore::default());
    let reply = get(
        app(Some(Arc::<FakeStore>::clone(&store)), DEFAULT_THRESHOLD),
        Some(&caller()),
        "/model/metrics/exceptions?_selected_model_group=chat",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, EMPTY_EXCEPTIONS);
    assert_eq!(
        store.calls(),
        vec![Call::Exceptions(ExceptionWindow {
            model_group: Some("chat".to_owned()),
            start: days_before(fixed_now(), 28),
            end: fixed_now(),
        })]
    );
}

#[tokio::test]
async fn oneshot_a_bad_start_time_is_400_and_reaches_no_store() {
    let store = Arc::new(FakeStore::default());
    let reply = get(
        app(Some(Arc::<FakeStore>::clone(&store)), DEFAULT_THRESHOLD),
        Some(&caller()),
        "/model/metrics?startTime=yesterday&endTime=2026-03-15",
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.body,
        r#"{"detail":{"error":"startTime must be an ISO 8601 datetime, got \"yesterday\""}}"#
    );
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn oneshot_a_bad_end_time_names_the_end_time() {
    let store = Arc::new(FakeStore::default());
    let reply = get(
        app(Some(Arc::<FakeStore>::clone(&store)), DEFAULT_THRESHOLD),
        Some(&caller()),
        "/model/metrics?endTime=2026-13-01",
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.body,
        r#"{"detail":{"error":"endTime must be an ISO 8601 datetime, got \"2026-13-01\""}}"#
    );
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn oneshot_store_failures_reach_the_generic_internal_error() {
    let mut fake = FakeStore::default();
    fake.fault = Some(Fault::CreateNull);
    let reply = get(
        app(Some(Arc::new(fake)), DEFAULT_THRESHOLD),
        Some(&caller()),
        STREAMING_METRICS_PATH,
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.body, INTERNAL_BODY);
}

#[tokio::test]
async fn oneshot_a_database_outage_answers_no_db_connection() {
    let mut fake = FakeStore::default();
    fake.fault = Some(Fault::PoolClosed);
    let reply = get(
        app(Some(Arc::new(fake)), DEFAULT_THRESHOLD),
        Some(&caller()),
        STREAMING_METRICS_PATH,
    )
    .await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.body, NO_DB_CONNECTION_BODY);
}

struct EarliestClock;

impl GatewayRequestClock for EarliestClock {
    fn utc_now(&self) -> DateTime<Utc> {
        DateTime::<Utc>::MIN_UTC
    }
}

#[test]
fn query_an_offset_datetime_is_read_in_utc() {
    let request = MetricsQuery::decode(Some(
        "startTime=2026-03-15T02:00:00%2B02:00&endTime=2026-03-15T23:30:00Z",
    ));
    let window = request.window(clock().as_ref(), 7).expect("the window");
    assert_eq!(window.start, instant("2026-03-15 00:00:00"));
    assert_eq!(window.end, fixed_now());
    assert_eq!(window.start.date_naive(), window.end.date_naive());
}

#[test]
fn query_a_date_only_value_is_midnight_utc() {
    let request = MetricsQuery::decode(Some("startTime=2026-03-15&endTime=2026-03-15T12:00:00"));
    let window = request.window(clock().as_ref(), 7).expect("the window");
    assert_eq!(window.start, instant("2026-03-15 00:00:00"));
    assert_eq!(window.end, instant("2026-03-15 12:00:00"));
    assert_eq!(window.start.date_naive(), window.end.date_naive());
}

#[test]
fn query_a_space_separated_datetime_is_read() {
    let request = MetricsQuery::decode(Some("startTime=2026-03-15%2012:00:00"));
    let window = request.window(clock().as_ref(), 7).expect("the window");
    assert_eq!(window.start, instant("2026-03-15 12:00:00"));
    assert_eq!(window.end, fixed_now());
}

#[rstest]
#[case::minutes_after_t("2026-03-15T12:00", instant("2026-03-15 12:00:00"))]
#[case::minutes_after_a_space("2026-03-15%2012:00", instant("2026-03-15 12:00:00"))]
#[case::underscore_separator("2026-03-15_12:00:00", instant("2026-03-15 12:00:00"))]
#[case::minutes_with_an_offset("2026-03-15T14:00%2B02:00", instant("2026-03-15 12:00:00"))]
#[case::unix_timestamp("20260315", instant("1970-08-23 11:51:55"))]
#[case::nine_fractional_digits(
    "2026-03-15T12:00:00.123456789",
    DateTime::parse_from_rfc3339("2026-03-15T12:00:00.123456Z")
        .expect("the expected instant")
        .with_timezone(&Utc)
)]
fn query_a_pydantic_datetime_form_is_read(#[case] text: &str, #[case] expected: DateTime<Utc>) {
    let request = MetricsQuery::decode(Some(&format!("startTime={text}")));
    let window = request.window(clock().as_ref(), 7).expect("the window");
    assert_eq!(window.start, expected, "{text}");
}

#[test]
fn query_without_times_uses_the_clock_and_the_lookback() {
    let request = MetricsQuery::decode(None);
    let window = request.window(clock().as_ref(), 7).expect("the window");
    assert_eq!(window.start, days_before(fixed_now(), 7));
    assert_eq!(window.end, fixed_now());
    assert_ne!(window.start.date_naive(), window.end.date_naive());
}

#[test]
fn query_a_start_before_the_calendar_start_stays_at_the_end() {
    let request = MetricsQuery::decode(None);
    let window = request.window(&EarliestClock, 7).expect("the window");
    assert_eq!(window.start, DateTime::<Utc>::MIN_UTC);
    assert_eq!(window.end, DateTime::<Utc>::MIN_UTC);
}

#[test]
fn query_repeats_take_the_last_value_and_unknown_names_are_ignored() {
    let request = MetricsQuery::decode(Some(
        "_selected_model_group=a&_selected_model_group=b&models=x&startTime=2026-03-01",
    ));
    assert_eq!(request.model_group, Some("b".to_owned()));
}

#[test]
fn query_a_missing_or_undefined_key_and_customer_are_null() {
    assert_eq!(normalized(None), "null");
    assert_eq!(normalized(Some("undefined")), "null");
    assert_eq!(normalized(Some("")), "");
    assert_eq!(normalized(Some("alice")), "alice");
    let decoded = MetricsQuery::decode(Some("api_key=undefined&customer=alice"));
    assert_eq!(decoded.api_key.as_deref(), Some("undefined"));
    assert_eq!(decoded.customer.as_deref(), Some("alice"));
    assert_eq!(MetricsQuery::decode(None).api_key.as_deref(), None);
}

#[test]
fn debug_spend_filters_redacts_the_key() {
    let filters = SpendFilters {
        model_group: "chat".to_owned(),
        start: instant("2026-03-01 00:00:00"),
        end: instant("2026-03-10 00:00:00"),
        api_key: "sk-secret".to_owned(),
        customer: "alice".to_owned(),
    };
    let rendered = format!("{filters:?}");
    assert!(!rendered.contains("sk-secret"));
    assert!(rendered.contains("api_key"));
    assert!(rendered.contains("***"));
}

#[test]
fn query_the_model_group_default_applies_only_when_absent() {
    assert_eq!(
        MetricsQuery::decode(None).model_group_or("gpt-4-32k"),
        "gpt-4-32k"
    );
    assert_eq!(
        MetricsQuery::decode(Some("_selected_model_group=")).model_group_or("gpt-4-32k"),
        ""
    );
}
