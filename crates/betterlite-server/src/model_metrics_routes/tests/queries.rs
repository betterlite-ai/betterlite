#![cfg(test)]

use super::support::database;
use super::support::execute;
use super::support::instant;
use super::support::postgres_store;
use crate::model_metrics_routes::store::ExceptionWindow;
use crate::model_metrics_routes::store::ModelMetricsStore as _;
use crate::model_metrics_routes::store::SlowRequest;
use crate::model_metrics_routes::store::SpendFilters;
use crate::model_metrics_routes::store::StreamingWindow;

const SAME_DAY_SEED: &str = r#"
INSERT INTO "LiteLLM_SpendLogs" (request_id, model, model_group, api_base, "startTime", "endTime", "completionStartTime", cache_hit) VALUES
    ('r1', 'm', 'g', 'https://a', '2026-03-15 10:00:00', '2026-03-15 10:00:05', '2026-03-15 10:00:01', 'False'),
    ('r2', 'm', 'g', 'https://a', '2026-03-15 10:00:00', '2026-03-15 10:00:05', '2026-03-15 10:00:01', 'True'),
    ('r3', 'm', 'g', 'https://a', '2026-03-15 10:00:00', '2026-03-15 10:00:05', NULL, 'False'),
    ('r4', 'm', 'g', 'https://a', '2026-03-15 10:00:00', '2026-03-15 10:00:05', '2026-03-15 10:00:05', 'False'),
    ('r5', 'm', 'g', 'https://a', '2026-03-14 10:00:00', '2026-03-14 10:00:05', '2026-03-14 10:00:02', 'False'),
    ('r6', 'm', 'other', 'https://a', '2026-03-15 10:00:00', '2026-03-15 10:00:05', '2026-03-15 10:00:02', 'False'),
    ('r7', 'm2', 'g', NULL, '2026-03-15 11:00:00', '2026-03-15 11:00:05', '2026-03-15 11:00:02', 'False');
"#;

const DAILY_SEED: &str = r#"
INSERT INTO "LiteLLM_SpendLogs" (request_id, model, model_group, api_base, "startTime", "endTime", "completionStartTime", cache_hit) VALUES
    ('d1', 'm', 'g', 'https://a', '2026-03-01 10:00:00', '2026-03-01 10:00:09', '2026-03-01 10:00:01', 'False'),
    ('d2', 'm', 'g', 'https://a', '2026-03-01 12:00:00', '2026-03-01 12:00:09', '2026-03-01 12:00:03', 'False'),
    ('d3', 'm', 'g', 'https://a', '2026-03-02 10:00:00', '2026-03-02 10:00:09', '2026-03-02 10:00:04', 'False'),
    ('d4', 'm', 'g', 'https://a', '2026-02-28 10:00:00', '2026-02-28 10:00:09', '2026-02-28 10:00:01', 'False'),
    ('d5', 'm', 'g', 'https://a', '2026-03-01 13:00:00', '2026-03-01 13:00:09', '2026-03-01 13:00:01', 'True'),
    ('d6', 'm', 'g', 'https://a', '2026-03-01 14:00:00', '2026-03-01 14:00:09', '2026-03-01 14:00:09', 'False'),
    ('d7', 'm', 'g', 'https://a', '2026-03-02 23:59:59', '2026-03-03 00:00:09', '2026-03-03 00:00:04', 'False'),
    ('d8', 'm', 'g', 'https://a', '2026-03-03 00:00:00', '2026-03-03 00:00:08', '2026-03-03 00:00:08', 'False'),
    ('d9', 'm', 'other', 'https://a', '2026-03-01 15:00:00', '2026-03-01 15:00:09', '2026-03-01 15:00:08', 'False'),
    ('d10', 'm', 'g', 'https://b', '2026-03-02 08:00:00', '2026-03-02 08:00:09', NULL, 'False'),
    ('d11', 'm', 'g', 'https://a', '2026-03-03 10:00:00', '2026-03-03 10:00:09', '2026-03-03 10:00:02', 'False');
"#;

const LATENCY_SEED: &str = r#"
INSERT INTO "LiteLLM_SpendLogs" (request_id, model, model_group, api_base, "startTime", "endTime", cache_hit, completion_tokens, api_key, end_user) VALUES
    ('l1', 'm', 'g', 'https://a', '2026-03-10 10:00:00', '2026-03-10 10:00:10', 'False', 5, 'k1', 'u1'),
    ('l2', 'm', 'g', 'https://a', '2026-03-10 11:00:00', '2026-03-10 11:00:06', 'False', 3, 'k1', 'u1'),
    ('l3', 'm', 'g', 'https://a', '2026-03-10 12:00:00', '2026-03-10 12:00:08', 'False', 0, 'k1', 'u1'),
    ('l4', 'm', 'g', 'https://a', '2026-03-10 13:00:00', '2026-03-10 13:00:10', 'True', 5, 'k1', 'u1'),
    ('l5', 'm', 'g', 'https://a', '2026-03-10 14:00:00', '2026-03-10 14:00:04', 'False', 1, 'k2', 'u1'),
    ('l6', 'm', 'g', 'https://a', '2026-03-10 15:00:00', '2026-03-10 15:00:08', 'False', 4, 'k1', 'u2'),
    ('l7', 'm', 'other', 'https://a', '2026-03-10 16:00:00', '2026-03-10 16:00:10', 'False', 5, 'k1', 'u1'),
    ('l8', 'm', 'g', 'https://a', '2026-02-28 10:00:00', '2026-02-28 10:00:10', 'False', 5, 'k1', 'u1'),
    ('l9', 'm', 'g', 'https://a', '2026-03-01 00:00:00', '2026-03-01 00:00:10', 'False', 2, 'k1', 'u1'),
    ('l10', 'm', 'g', 'https://a', '2026-03-10 23:59:59', '2026-03-11 00:00:09', 'False', 4, 'k1', 'u3'),
    ('l11', 'm', 'g', 'https://a', '2026-03-11 00:00:00', '2026-03-11 00:00:10', 'False', 5, 'k1', 'u1'),
    ('z1', 'm', 'g', 'https://a', '2026-03-10 17:00:00', '2026-03-10 17:00:02', 'False', 0, 'k1', 'u1'),
    ('z2', 'm', 'g', 'https://a', '2026-03-05 10:00:00', '2026-03-05 10:00:10', 'False', 0, 'k1', 'u1');
"#;

const SLOW_SEED: &str = r#"
INSERT INTO "LiteLLM_SpendLogs" (request_id, model, model_group, api_base, "startTime", "endTime", cache_hit, api_key, end_user) VALUES
    ('s1', 'm', 'g', 'https://a', '2026-03-10 10:00:00', '2026-03-10 10:00:05', 'False', 'k1', 'u1'),
    ('s2', 'm', 'g', 'https://a', '2026-03-10 11:00:00', '2026-03-10 11:00:10', 'False', 'k1', 'u1'),
    ('s3', 'm', 'g', 'https://b', '2026-03-10 12:00:00', '2026-03-10 12:00:04', 'False', 'k1', 'u1'),
    ('s4', 'm', 'g', NULL, '2026-03-10 13:00:00', '2026-03-10 13:00:07', 'False', 'k1', 'u1'),
    ('s5', 'm', 'g', 'https://a', '2026-03-10 14:00:00', '2026-03-10 14:00:20', 'True', 'k1', 'u1'),
    ('s6', 'm', 'g', 'https://a', '2026-03-10 15:00:00', '2026-03-10 15:00:20', 'False', 'k2', 'u1'),
    ('s7', 'm', 'g', 'https://a', '2026-03-10 16:00:00', '2026-03-10 16:00:20', 'False', 'k1', 'u2'),
    ('s8', 'm', 'other', 'https://a', '2026-03-10 17:00:00', '2026-03-10 17:00:20', 'False', 'k1', 'u1'),
    ('s9', 'm', 'g', 'https://a', '2026-03-01 00:00:00', '2026-03-01 00:00:10', 'False', 'k1', 'u1'),
    ('s10', 'm', 'g', 'https://a', '2026-03-10 23:59:59', '2026-03-11 00:00:04', 'False', 'k1', 'u1'),
    ('s11', 'm', 'g', 'https://d', '2026-02-28 23:59:59', '2026-03-01 00:00:09', 'False', 'k1', 'u1'),
    ('s12', 'm', 'g', 'https://e', '2026-03-11 00:00:00', '2026-03-11 00:00:20', 'False', 'k1', 'u1');
"#;

const EXCEPTIONS_SEED: &str = r#"
INSERT INTO "LiteLLM_ErrorLogs" (request_id, "startTime", "endTime", api_base, model_group, litellm_model_name, exception_type) VALUES
    ('e1', '2026-03-10 10:00:00', '2026-03-10 10:00:01', 'https://a', 'g', 'gpt', 'RateLimitError'),
    ('e2', '2026-03-10 10:00:00', '2026-03-10 10:00:01', 'https://a', 'g', 'gpt', 'RateLimitError'),
    ('e3', '2026-03-10 10:00:00', '2026-03-10 10:00:01', 'https://a', 'g', 'gpt', 'AuthError'),
    ('e4', '2026-03-10 10:00:00', '2026-03-10 10:00:01', '', 'g', 'gpt', 'TimeoutError'),
    ('e5', '2026-03-10 10:00:00', '2026-03-10 10:00:01', 'https://a', 'other', 'gpt', 'RateLimitError'),
    ('e6', '2026-03-09 10:00:00', '2026-03-10 13:00:00', 'https://a', 'g', 'gpt', 'RateLimitError'),
    ('e7', '2026-02-28 10:00:00', '2026-02-28 10:00:01', 'https://a', 'g', 'gpt', 'RateLimitError'),
    ('e8', '2026-03-10 11:59:00', '2026-03-10 12:00:00', 'https://a', 'g', 'gpt', 'TimeoutError');
"#;

const CAP_SEED: &str = r#"
INSERT INTO "LiteLLM_ErrorLogs" (request_id, "startTime", "endTime", api_base, model_group, litellm_model_name, exception_type)
SELECT 'cap-' || i, '2026-03-10 10:00:00', '2026-03-10 10:00:01', '', 'g', 'model-' || lpad(i::text, 3, '0'), 'RateLimitError'
FROM generate_series(1, 201) AS i;
"#;

#[tokio::test]
async fn query_same_day_lists_one_row_per_request_and_skips_excluded_rows() {
    let (fixture, pool) = database().await;
    execute(&pool, SAME_DAY_SEED).await;
    let rows = postgres_store(&pool)
        .streaming_same_day(&StreamingWindow {
            model_group: Some("g".to_owned()),
            start: instant("2026-03-15 00:00:00"),
            end: instant("2026-03-15 23:59:59"),
        })
        .await
        .expect("the same-day rows");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].key, "r7");
    assert_eq!(rows[0].api_base.as_deref(), None);
    assert_eq!(rows[0].model, "m2");
    assert!(rows[0].value.total_cmp(&2.0_f64).is_eq());
    assert_eq!(rows[1].key, "r1");
    assert_eq!(rows[1].api_base.as_deref(), Some("https://a"));
    assert_eq!(rows[1].model, "m");
    assert!(rows[1].value.total_cmp(&1.0_f64).is_eq());
    pool.close();
    fixture.close_async().await.expect("the fixture close");
}

#[tokio::test]
async fn query_daily_averages_the_first_token_per_day_inside_the_window() {
    let (fixture, pool) = database().await;
    execute(&pool, DAILY_SEED).await;
    let rows = postgres_store(&pool)
        .streaming_daily(&StreamingWindow {
            model_group: Some("g".to_owned()),
            start: instant("2026-03-01 00:00:00"),
            end: instant("2026-03-02 23:59:59"),
        })
        .await
        .expect("the daily rows");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].key, "2026-03-02");
    assert_eq!(rows[0].api_base.as_deref(), Some("https://a"));
    assert!(rows[0].value.total_cmp(&4.5_f64).is_eq());
    assert_eq!(rows[1].key, "2026-03-01");
    assert!(rows[1].value.total_cmp(&2.0_f64).is_eq());
    pool.close();
    fixture.close_async().await.expect("the fixture close");
}

#[tokio::test]
async fn query_latency_divides_seconds_by_completion_tokens_and_filters() {
    let (fixture, pool) = database().await;
    execute(&pool, LATENCY_SEED).await;
    let store = postgres_store(&pool);
    let filtered = store
        .latency(&SpendFilters {
            model_group: "g".to_owned(),
            start: instant("2026-03-01 00:00:00"),
            end: instant("2026-03-10 23:59:59"),
            api_key: "k1".to_owned(),
            customer: "u1".to_owned(),
        })
        .await
        .expect("the filtered latency rows");
    assert_eq!(filtered.len(), 2);
    assert_eq!(filtered[0].key, "2026-03-01");
    assert!(filtered[0].value.total_cmp(&5.0_f64).is_eq());
    assert_eq!(filtered[1].key, "2026-03-10");
    assert!(filtered[1].value.total_cmp(&2.0_f64).is_eq());
    let unfiltered = store
        .latency(&SpendFilters {
            model_group: "g".to_owned(),
            start: instant("2026-03-01 00:00:00"),
            end: instant("2026-03-10 23:59:59"),
            api_key: "null".to_owned(),
            customer: "null".to_owned(),
        })
        .await
        .expect("the unfiltered latency rows");
    assert_eq!(unfiltered.len(), 2);
    assert_eq!(unfiltered[0].key, "2026-03-01");
    assert!(unfiltered[0].value.total_cmp(&5.0_f64).is_eq());
    assert_eq!(unfiltered[1].key, "2026-03-10");
    assert!(unfiltered[1].value.total_cmp(&2.5_f64).is_eq());
    pool.close();
    fixture.close_async().await.expect("the fixture close");
}

#[tokio::test]
async fn query_slow_responses_counts_at_the_threshold_and_orders_by_slow_count() {
    let (fixture, pool) = database().await;
    execute(&pool, SLOW_SEED).await;
    let rows = postgres_store(&pool)
        .slow_responses(&SlowRequest {
            filters: SpendFilters {
                model_group: "g".to_owned(),
                start: instant("2026-03-01 00:00:00"),
                end: instant("2026-03-10 23:59:59"),
                api_key: "k1".to_owned(),
                customer: "u1".to_owned(),
            },
            threshold_seconds: 5,
        })
        .await
        .expect("the slow rows");
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].api_base.as_deref(), Some("https://a"));
    assert_eq!(rows[0].total_count, 4);
    assert_eq!(rows[0].slow_count, 4);
    assert_eq!(rows[1].api_base.as_deref(), None);
    assert_eq!(rows[1].total_count, 1);
    assert_eq!(rows[1].slow_count, 1);
    assert_eq!(rows[2].api_base.as_deref(), Some("https://b"));
    assert_eq!(rows[2].total_count, 1);
    assert_eq!(rows[2].slow_count, 0);
    pool.close();
    fixture.close_async().await.expect("the fixture close");
}

#[tokio::test]
async fn query_slow_responses_with_null_filters_counts_every_base() {
    let (fixture, pool) = database().await;
    execute(&pool, SLOW_SEED).await;
    let rows = postgres_store(&pool)
        .slow_responses(&SlowRequest {
            filters: SpendFilters {
                model_group: "g".to_owned(),
                start: instant("2026-03-01 00:00:00"),
                end: instant("2026-03-10 23:59:59"),
                api_key: "null".to_owned(),
                customer: "null".to_owned(),
            },
            threshold_seconds: 5,
        })
        .await
        .expect("the unfiltered slow rows");
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].api_base.as_deref(), Some("https://a"));
    assert_eq!(rows[0].total_count, 6);
    assert_eq!(rows[0].slow_count, 6);
    assert_eq!(rows[1].api_base.as_deref(), None);
    assert_eq!(rows[1].total_count, 1);
    assert_eq!(rows[1].slow_count, 1);
    assert_eq!(rows[2].api_base.as_deref(), Some("https://b"));
    assert_eq!(rows[2].total_count, 1);
    assert_eq!(rows[2].slow_count, 0);
    pool.close();
    fixture.close_async().await.expect("the fixture close");
}

#[tokio::test]
async fn query_exceptions_joins_the_model_and_api_base_and_compares_end_time() {
    let (fixture, pool) = database().await;
    execute(&pool, EXCEPTIONS_SEED).await;
    let store = postgres_store(&pool);
    let window = ExceptionWindow {
        model_group: Some("g".to_owned()),
        start: instant("2026-03-01 00:00:00"),
        end: instant("2026-03-10 12:00:00"),
    };
    let rows = store.exceptions(&window).await.expect("the exception rows");
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].model, "gpt-https://a");
    assert_eq!(rows[0].exception_type, "AuthError");
    assert_eq!(rows[0].count, 1);
    assert_eq!(rows[1].model, "gpt-https://a");
    assert_eq!(rows[1].exception_type, "RateLimitError");
    assert_eq!(rows[1].count, 2);
    assert_eq!(rows[2].model, "gpt-https://a");
    assert_eq!(rows[2].exception_type, "TimeoutError");
    assert_eq!(rows[2].count, 1);
    assert_eq!(rows[3].model, "gpt");
    assert_eq!(rows[3].exception_type, "TimeoutError");
    assert_eq!(rows[3].count, 1);
    let unselected = store
        .exceptions(&ExceptionWindow {
            model_group: None,
            start: instant("2026-03-01 00:00:00"),
            end: instant("2026-03-10 12:00:00"),
        })
        .await
        .expect("the unselected exception rows");
    assert!(unselected.is_empty());
    pool.close();
    fixture.close_async().await.expect("the fixture close");
}

#[tokio::test]
async fn query_exceptions_caps_the_models_at_two_hundred() {
    let (fixture, pool) = database().await;
    execute(&pool, CAP_SEED).await;
    let rows = postgres_store(&pool)
        .exceptions(&ExceptionWindow {
            model_group: Some("g".to_owned()),
            start: instant("2026-03-01 00:00:00"),
            end: instant("2026-03-10 12:00:00"),
        })
        .await
        .expect("the capped exception rows");
    assert_eq!(rows.len(), 200);
    assert_eq!(rows[0].model, "model-001");
    assert_eq!(rows[199].model, "model-200");
    pool.close();
    fixture.close_async().await.expect("the fixture close");
}
