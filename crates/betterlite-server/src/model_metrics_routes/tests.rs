#![cfg(test)]

mod queries;
mod support;

use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_db::gateway_request_tracking::GatewayRequestClock;
use chrono::DateTime;
use chrono::NaiveDate;
use chrono::TimeDelta;
use chrono::Utc;

use super::EXCEPTIONS_PATH;
use super::METRICS_PATH;
use super::SLOW_RESPONSES_PATH;
use super::STREAMING_METRICS_PATH;
use super::query::MetricsQuery;
use super::query::normalized;
use super::store::DailyRow;
use super::store::ExceptionRow;
use super::store::ExceptionWindow;
use super::store::LatencyRequest;
use super::store::LatencyRow;
use super::store::SameDayRow;
use super::store::SlowRequest;
use super::store::SlowRow;
use super::store::StreamingWindow;
use support::Call;
use support::DEFAULT_THRESHOLD;
use support::FakeStore;
use support::app;
use support::caller;
use support::clock;
use support::fixed_now;
use support::get;
use support::instant;

const UNAUTHORIZED_BODY: &str = r#"{"error":{"message":"the API key is not valid","type":"auth_error","param":"None","code":"401"}}"#;

const DB_NOT_CONNECTED_BODY: &str = r#"{"error":{"message":"DB not connected. This endpoint needs a database; set DATABASE_URL to a PostgreSQL connection string (postgresql://...) to enable it. See https://docs.litellm.ai/docs/proxy/virtual_keys","type":"internal_error","param":"None","code":"500"}}"#;

const PRISMA_BODY: &str = r#"{"error":{"message":"Prisma Client is not initialized","type":"internal_error","param":"None","code":"500"}}"#;

const INTERNAL_BODY: &str = r#"{"error":{"message":"Internal server error","type":"internal_server_error","param":null,"code":"500"}}"#;

const EMPTY_SERIES: &str = r#"{"data":[],"all_api_bases":[]}"#;

const EMPTY_EXCEPTIONS: &str = r#"{"data":[],"exception_types":[]}"#;

fn day(year: i32, month: u32, day_of_month: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day_of_month).expect("the calendar date")
}

#[tokio::test]
async fn oneshot_every_route_without_a_key_is_401() {
    for path in [
        STREAMING_METRICS_PATH,
        METRICS_PATH,
        SLOW_RESPONSES_PATH,
        EXCEPTIONS_PATH,
    ] {
        let reply = get(app(None, DEFAULT_THRESHOLD), None, path).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{path}");
        assert_eq!(reply.body, UNAUTHORIZED_BODY, "{path}");
    }
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
            start: fixed_now() - TimeDelta::days(7),
            end: fixed_now(),
        })]
    );
}

#[tokio::test]
async fn oneshot_streaming_metrics_a_single_day_lists_each_request() {
    let mut fake = FakeStore::default();
    fake.rows.same_day = vec![
        SameDayRow {
            api_base: Some("https://api.example.com/v1/openai/".to_owned()),
            model: "gpt-4".to_owned(),
            request_id: "req-b".to_owned(),
            time_to_first_token: 0.25,
        },
        SameDayRow {
            api_base: None,
            model: "gpt-4".to_owned(),
            request_id: "req-a".to_owned(),
            time_to_first_token: 0.5,
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
            start: instant(2026, 3, 15, 0, 0, 0),
            end: instant(2026, 3, 15, 23, 59, 59),
        })]
    );
}

#[tokio::test]
async fn oneshot_streaming_metrics_several_days_averages_per_day() {
    let mut fake = FakeStore::default();
    fake.rows.daily = vec![
        DailyRow {
            api_base: None,
            model: "gpt-4".to_owned(),
            day: day(2026, 3, 2),
            time_to_first_token: 0.125,
        },
        DailyRow {
            api_base: Some("https://api.example.com/v1".to_owned()),
            model: "gpt-4".to_owned(),
            day: day(2026, 3, 1),
            time_to_first_token: 0.5,
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
            start: instant(2026, 3, 1, 0, 0, 0),
            end: instant(2026, 3, 10, 0, 0, 0),
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
            start: instant(2026, 3, 1, 0, 0, 0),
            end: fixed_now(),
        })]
    );
}

#[tokio::test]
async fn oneshot_streaming_metrics_takes_the_start_from_the_end_when_only_the_end_is_given() {
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
            start: instant(2026, 3, 3, 0, 0, 0),
            end: instant(2026, 3, 10, 0, 0, 0),
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
        vec![Call::Latency(LatencyRequest {
            model_group: "gpt-4-32k".to_owned(),
            start: fixed_now() - TimeDelta::days(28),
            end: fixed_now(),
            api_key: "null".to_owned(),
            customer: "null".to_owned(),
        })]
    );
}

#[tokio::test]
async fn oneshot_model_metrics_sends_the_filters_and_names_bases() {
    let mut fake = FakeStore::default();
    fake.rows.latency = vec![
        LatencyRow {
            api_base: Some("http://plain.example.com".to_owned()),
            model: "claude".to_owned(),
            day: day(2026, 3, 11),
            avg_latency_per_token: 0.008,
        },
        LatencyRow {
            api_base: Some("https://api.example.com/openai/v1".to_owned()),
            model: "gpt-4".to_owned(),
            day: day(2026, 3, 10),
            avg_latency_per_token: 0.002,
        },
        LatencyRow {
            api_base: None,
            model: "claude".to_owned(),
            day: day(2026, 3, 10),
            avg_latency_per_token: 0.004,
        },
    ];
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
        vec![Call::Latency(LatencyRequest {
            model_group: "chat".to_owned(),
            start: fixed_now() - TimeDelta::days(28),
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
        vec![Call::Latency(LatencyRequest {
            model_group: "gpt-4-32k".to_owned(),
            start: fixed_now() - TimeDelta::days(28),
            end: fixed_now(),
            api_key: "null".to_owned(),
            customer: "null".to_owned(),
        })]
    );
}

#[tokio::test]
async fn oneshot_slow_responses_truncates_the_threshold() {
    for (configured, expected) in [(300.75_f64, 300_i64), (12.9, 12), (-1.5, -1), (0.0, 0)] {
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
                model_group: "gpt-4-32k".to_owned(),
                start: fixed_now() - TimeDelta::days(28),
                end: fixed_now(),
                api_key: "null".to_owned(),
                customer: "null".to_owned(),
                alerting_threshold_seconds: expected,
            })],
            "{configured}"
        );
    }
}

#[tokio::test]
async fn oneshot_slow_responses_a_threshold_that_is_not_whole_is_500() {
    for configured in [f64::NAN, f64::INFINITY] {
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
            model_group: "chat".to_owned(),
            start: fixed_now() - TimeDelta::days(28),
            end: fixed_now(),
            api_key: "sk-1".to_owned(),
            customer: "alice".to_owned(),
            alerting_threshold_seconds: 300,
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
            start: fixed_now() - TimeDelta::days(28),
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
            start: fixed_now() - TimeDelta::days(28),
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
    fake.failure = true;
    let reply = get(
        app(Some(Arc::new(fake)), DEFAULT_THRESHOLD),
        Some(&caller()),
        STREAMING_METRICS_PATH,
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.body, INTERNAL_BODY);
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
    assert_eq!(window.start, instant(2026, 3, 15, 0, 0, 0));
    assert_eq!(window.end, fixed_now());
    assert_eq!(window.start.date_naive(), window.end.date_naive());
}

#[test]
fn query_a_date_only_value_is_midnight_utc() {
    let request = MetricsQuery::decode(Some("startTime=2026-03-15&endTime=2026-03-15T12:00:00"));
    let window = request.window(clock().as_ref(), 7).expect("the window");
    assert_eq!(window.start, instant(2026, 3, 15, 0, 0, 0));
    assert_eq!(window.end, instant(2026, 3, 15, 12, 0, 0));
    assert_eq!(window.start.date_naive(), window.end.date_naive());
}

#[test]
fn query_a_space_separated_datetime_is_read() {
    let request = MetricsQuery::decode(Some("startTime=2026-03-15%2012:00:00"));
    let window = request.window(clock().as_ref(), 7).expect("the window");
    assert_eq!(window.start, instant(2026, 3, 15, 12, 0, 0));
    assert_eq!(window.end, fixed_now());
}

#[test]
fn query_without_times_uses_the_clock_and_the_lookback() {
    let request = MetricsQuery::decode(None);
    let window = request.window(clock().as_ref(), 7).expect("the window");
    assert_eq!(window.start, fixed_now() - TimeDelta::days(7));
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
