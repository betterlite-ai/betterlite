#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::{DeploymentMetrics, metric_keys, metric_number, top_five, update_daily_reports};
use crate::alerting::deployment_reports::tests::support::{
    FakeCache, Increment, fixed_now, float_metric, int_metric,
};

#[test]
fn the_metric_keys_follow_the_cache_key_enum() {
    let keys = metric_keys(&["d1".to_owned(), "d2".to_owned()]);

    assert_eq!(
        keys.failed,
        vec![
            "d1:failed_requests_daily_metrics".to_owned(),
            "d2:failed_requests_daily_metrics".to_owned()
        ]
    );
    assert_eq!(
        keys.latency,
        vec![
            "d1:latency_daily_metrics".to_owned(),
            "d2:latency_daily_metrics".to_owned()
        ]
    );
    assert_eq!(
        keys.combined,
        vec![
            "d1:failed_requests_daily_metrics".to_owned(),
            "d2:failed_requests_daily_metrics".to_owned(),
            "d1:latency_daily_metrics".to_owned(),
            "d2:latency_daily_metrics".to_owned()
        ]
    );
}

#[rstest]
#[case::float(float_metric(1.5), Some(1.5_f64))]
#[case::integer(int_metric(2), Some(2.0_f64))]
#[case::null(Json::Null, None)]
#[case::text(Json::Text("2".to_owned()), None)]
#[case::boolean(Json::Bool(true), None)]
#[case::list(Json::List(Vec::new()), None)]
#[case::object(Json::Object(IndexMap::new()), None)]
fn a_metric_is_a_json_number(#[case] value: Json, #[case] expected: Option<f64>) {
    assert_eq!(
        metric_number(&value).map(f64::to_bits),
        expected.map(f64::to_bits)
    );
}

#[rstest]
#[case::highest_first(
    vec![Some(1.0_f64), Some(3.0_f64), Some(3.0_f64), Some(0.0_f64), Some(2.0_f64)],
    vec![1, 2, 4, 0]
)]
#[case::drops_zero_and_negative(vec![Some(-1.0_f64), Some(0.0_f64), Some(2.0_f64)], vec![2])]
#[case::missing_counts_as_zero(vec![None, Some(1.0_f64)], vec![1])]
#[case::five_at_most(
    vec![
        Some(6.0_f64),
        Some(5.0_f64),
        Some(4.0_f64),
        Some(3.0_f64),
        Some(2.0_f64),
        Some(1.0_f64)
    ],
    vec![0, 1, 2, 3, 4]
)]
#[case::all_zero(vec![Some(0.0_f64), None], Vec::new())]
fn the_top_five_ranks_the_positive_values(
    #[case] values: Vec<Option<f64>>,
    #[case] expected: Vec<usize>,
) {
    assert_eq!(top_five(&values), expected);
}

#[tokio::test]
async fn a_failed_request_increments_only_the_failed_counter() {
    let cache = FakeCache::default();
    let metrics = DeploymentMetrics {
        id: "d1",
        failed_request: true,
        latency_per_output_token: None,
        updated_at: fixed_now(),
    };

    let updated = update_daily_reports(&cache, metrics).await;

    assert_eq!(updated, 1);
    assert_eq!(
        cache.increments.entries(),
        vec![Increment {
            key: "d1:failed_requests_daily_metrics".to_owned(),
            amount: 1.0,
        }]
    );
}

#[tokio::test]
async fn a_latency_increments_only_the_latency_counter() {
    let cache = FakeCache::default();
    let metrics = DeploymentMetrics {
        id: "d1",
        failed_request: false,
        latency_per_output_token: Some(0.5_f64),
        updated_at: fixed_now(),
    };

    let updated = update_daily_reports(&cache, metrics).await;

    assert_eq!(updated, 1);
    assert_eq!(
        cache.increments.entries(),
        vec![Increment {
            key: "d1:latency_daily_metrics".to_owned(),
            amount: 0.5,
        }]
    );
}

#[tokio::test]
async fn both_metrics_increment_in_order_and_count_two() {
    let cache = FakeCache::default();
    let metrics = DeploymentMetrics {
        id: "d1",
        failed_request: true,
        latency_per_output_token: Some(0.25_f64),
        updated_at: fixed_now(),
    };

    let updated = update_daily_reports(&cache, metrics).await;

    assert_eq!(updated, 2);
    assert_eq!(
        cache.increments.entries(),
        vec![
            Increment {
                key: "d1:failed_requests_daily_metrics".to_owned(),
                amount: 1.0,
            },
            Increment {
                key: "d1:latency_daily_metrics".to_owned(),
                amount: 0.25,
            }
        ]
    );
}

#[tokio::test]
async fn a_record_with_neither_metric_updates_nothing() {
    let cache = FakeCache::default();
    let metrics = DeploymentMetrics {
        id: "d1",
        failed_request: false,
        latency_per_output_token: None,
        updated_at: fixed_now(),
    };

    let updated = update_daily_reports(&cache, metrics).await;

    assert_eq!(updated, 0);
    assert!(cache.increments.entries().is_empty());
}

#[rstest]
#[case::zero(0.0_f64)]
#[case::negative(-0.5_f64)]
#[tokio::test]
async fn a_latency_is_incremented_even_when_it_is_not_positive(#[case] latency: f64) {
    let cache = FakeCache::default();
    let metrics = DeploymentMetrics {
        id: "d1",
        failed_request: false,
        latency_per_output_token: Some(latency),
        updated_at: fixed_now(),
    };

    let updated = update_daily_reports(&cache, metrics).await;

    assert_eq!(updated, 1);
    assert_eq!(
        cache.increments.entries(),
        vec![Increment {
            key: "d1:latency_daily_metrics".to_owned(),
            amount: latency,
        }]
    );
}
