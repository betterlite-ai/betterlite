#![cfg(test)]

use std::sync::Arc;

use betterlite_prometheus::buckets::LATENCY_BUCKETS;
use betterlite_prometheus::collector_factory::collector::MetricValue;
use betterlite_prometheus::collector_registry::CollectorRegistry;
use betterlite_prometheus::collector_registry::metric::sample::Sample;
use betterlite_prometheus::collector_registry::metric::{
    Bucket, BucketError, CollectorKind, PrometheusMetric, prepare_buckets,
};
use rstest::rstest;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;

use crate::logging_service_events::types::wire_names;
use crate::logging_service_events::types::{
    ServiceEventMetadata, ServiceLoggerPayload, ServiceType,
};

use super::PrometheusServicesLogger;
use super::error::PrometheusServicesError;

fn build(buckets: Option<&[f64]>) -> (Arc<CollectorRegistry>, PrometheusServicesLogger) {
    let registry = Arc::new(CollectorRegistry::default());
    let logger = PrometheusServicesLogger::new(Arc::clone(&registry), buckets)
        .expect("the service logger builds");
    (registry, logger)
}

fn registered(registry: &CollectorRegistry, name: &str) -> PrometheusMetric {
    registry
        .get(name)
        .unwrap_or_else(|| panic!("{name} is registered"))
}

fn sample(metric: &PrometheusMetric, labels: &[Option<String>]) -> Sample {
    metric.series(labels).expect("the series exists")
}

fn series_count(metric: &PrometheusMetric) -> usize {
    metric.samples.lock().expect("the samples lock").len()
}

fn success(service: ServiceType, duration: f64) -> ServiceLoggerPayload {
    ServiceLoggerPayload {
        is_error: false,
        error: None,
        service,
        duration,
        call_type: "get_key_object".to_owned(),
        caller: None,
        event_metadata: None,
    }
}

fn failure(service: ServiceType, duration: f64) -> ServiceLoggerPayload {
    ServiceLoggerPayload {
        is_error: true,
        error: Some("connection refused".to_owned()),
        ..success(service, duration)
    }
}

#[rstest]
#[case::redis(ServiceType::Redis, "redis")]
#[case::postgres(ServiceType::Postgres, "postgres")]
#[case::batch_write_to_db(ServiceType::BatchWriteToDb, "batch_write_to_db")]
#[case::reset_budget_job(ServiceType::ResetBudgetJob, "reset_budget_job")]
#[case::self_service(ServiceType::Litellm, "self")]
#[case::router(ServiceType::Router, "router")]
#[case::auth(ServiceType::Auth, "auth")]
#[case::proxy_pre_call(ServiceType::ProxyPreCall, "proxy_pre_call")]
#[case::redis_daily_org_spend_update_queue(
    ServiceType::RedisDailyOrgSpendUpdateQueue,
    "redis_daily_org_spend_update_queue"
)]
#[case::redis_daily_team_spend_update_queue(
    ServiceType::RedisDailyTeamSpendUpdateQueue,
    "redis_daily_team_spend_update_queue"
)]
#[case::redis_daily_tag_spend_update_queue(
    ServiceType::RedisDailyTagSpendUpdateQueue,
    "redis_daily_tag_spend_update_queue"
)]
#[case::redis_window_spend_update_queue(
    ServiceType::RedisWindowSpendUpdateQueue,
    "redis_window_spend_update_queue"
)]
fn counter_services_register_latency_and_two_counters(
    #[case] service: ServiceType,
    #[case] name: &str,
) {
    let (registry, logger) = build(None);
    assert_eq!(wire_names::as_str(service), name);
    assert!(logger.payload_to_prometheus_map.contains_key(name));

    let histogram = registered(&registry, &format!("litellm_{name}_latency"));
    assert_eq!(histogram.description.kind, CollectorKind::Histogram);
    assert_eq!(histogram.description.label_names, vec![name.to_owned()]);

    let failed = registered(&registry, &format!("litellm_{name}_failed_requests"));
    assert_eq!(failed.description.kind, CollectorKind::Counter);
    assert_eq!(
        failed.description.label_names,
        vec![
            name.to_owned(),
            "error_class".to_owned(),
            "function_name".to_owned()
        ]
    );

    let total = registered(&registry, &format!("litellm_{name}_total_requests"));
    assert_eq!(total.description.kind, CollectorKind::Counter);
    assert_eq!(total.description.label_names, vec![name.to_owned()]);
}

#[rstest]
#[case::pod_lock_manager(ServiceType::PodLockManager, "pod_lock_manager")]
#[case::in_memory_daily_spend_update_queue(
    ServiceType::InMemoryDailySpendUpdateQueue,
    "in_memory_daily_spend_update_queue"
)]
#[case::redis_daily_spend_update_queue(
    ServiceType::RedisDailySpendUpdateQueue,
    "redis_daily_spend_update_queue"
)]
#[case::redis_daily_end_user_spend_update_queue(
    ServiceType::RedisDailyEndUserQueue,
    "redis_daily_end_user_spend_update_queue"
)]
#[case::redis_daily_agent_spend_update_queue(
    ServiceType::RedisDailyAgentQueue,
    "redis_daily_agent_spend_update_queue"
)]
#[case::in_memory_spend_update_queue(
    ServiceType::InMemorySpendUpdateQueue,
    "in_memory_spend_update_queue"
)]
#[case::redis_spend_update_queue(ServiceType::RedisSpendUpdateQueue, "redis_spend_update_queue")]
fn gauge_services_register_only_a_size_gauge(#[case] service: ServiceType, #[case] name: &str) {
    let (registry, logger) = build(None);
    assert_eq!(wire_names::as_str(service), name);

    let gauge = registered(&registry, &format!("litellm_{name}_size"));
    assert_eq!(gauge.description.kind, CollectorKind::Gauge);
    assert_eq!(gauge.description.label_names, vec![name.to_owned()]);
    assert!(registry.get(&format!("litellm_{name}_latency")).is_none());
    assert!(
        registry
            .get(&format!("litellm_{name}_failed_requests"))
            .is_none()
    );
    assert!(
        registry
            .get(&format!("litellm_{name}_total_requests"))
            .is_none()
    );
    assert_eq!(logger.payload_to_prometheus_map[name].len(), 1);
}

#[test]
fn every_service_type_gets_a_metric_list() {
    let (_, logger) = build(None);
    assert_eq!(
        logger.payload_to_prometheus_map.len(),
        ServiceType::ALL.len()
    );
    assert!(logger.payload_to_prometheus_map.contains_key("self"));
    assert!(logger.payload_to_prometheus_map.contains_key("postgres"));
    assert!(!logger.payload_to_prometheus_map.contains_key("litellm"));
}

#[test]
fn metric_help_texts_match_litellm() {
    let (registry, _logger) = build(None);
    assert_eq!(
        registered(&registry, "litellm_redis_latency")
            .description
            .help,
        "Latency for redis service"
    );
    assert_eq!(
        registered(&registry, "litellm_redis_failed_requests")
            .description
            .help,
        "Total failed_requests for redis service"
    );
    assert_eq!(
        registered(&registry, "litellm_redis_total_requests")
            .description
            .help,
        "Total total_requests for redis service"
    );
    assert_eq!(
        registered(&registry, "litellm_pod_lock_manager_size")
            .description
            .help,
        "Gauge for pod_lock_manager service"
    );
}

#[test]
fn the_default_latency_buckets_are_the_latency_table() {
    let (registry, _logger) = build(None);
    let buckets = registered(&registry, "litellm_redis_latency")
        .description
        .buckets;
    assert_eq!(buckets.len(), LATENCY_BUCKETS.len());
    assert_eq!(buckets.first(), Some(&Bucket::Seconds(0.005)));
    assert_eq!(buckets.get(15), Some(&Bucket::Seconds(420.0)));
    assert_eq!(buckets.last(), Some(&Bucket::Infinite));
}

#[test]
fn custom_latency_buckets_replace_the_default() {
    let (registry, _logger) = build(Some(&[0.5, 2.5]));
    assert_eq!(
        registered(&registry, "litellm_redis_latency")
            .description
            .buckets,
        vec![Bucket::Seconds(0.5), Bucket::Seconds(2.5), Bucket::Infinite]
    );
}

#[test]
fn unsorted_custom_buckets_fail_the_constructor() {
    let registry = Arc::new(CollectorRegistry::default());
    let error =
        PrometheusServicesLogger::new(Arc::clone(&registry), Some(&[2.5, 0.5])).unwrap_err();
    assert!(matches!(
        error,
        PrometheusServicesError::Buckets(BucketError::NotSorted)
    ));
    assert!(registry.metrics().is_empty());
}

#[test]
fn too_few_custom_buckets_fail_the_constructor() {
    let registry = Arc::new(CollectorRegistry::default());
    let error = PrometheusServicesLogger::new(Arc::clone(&registry), Some(&[])).unwrap_err();
    assert!(matches!(
        error,
        PrometheusServicesError::Buckets(BucketError::TooFew)
    ));
    assert!(registry.metrics().is_empty());
}

#[test]
fn a_second_logger_reuses_the_registered_collectors() {
    let registry = Arc::new(CollectorRegistry::default());
    let first = PrometheusServicesLogger::new(Arc::clone(&registry), None)
        .expect("the first logger builds");
    let descriptions = registry.descriptions().len();

    let second = PrometheusServicesLogger::new(Arc::clone(&registry), None)
        .expect("the second logger reuses the registered collectors");
    assert_eq!(registry.descriptions().len(), descriptions);

    let total = second
        .payload_to_prometheus_map
        .get("redis")
        .expect("redis has metrics")
        .iter()
        .find(|metric| metric.description.name == "litellm_redis_total_requests")
        .expect("the total requests counter");
    PrometheusServicesLogger::increment_counter(total, "redis", 1.0, &[]);
    assert_eq!(
        sample(
            &registered(&registry, "litellm_redis_total_requests"),
            &[Some("redis".to_owned())]
        )
        .total,
        MetricValue::Finite(Decimal::ONE)
    );
    assert_eq!(
        first.payload_to_prometheus_map.len(),
        second.payload_to_prometheus_map.len()
    );
}

#[test]
fn a_second_logger_skips_bucket_validation_for_reused_collectors() {
    let registry = Arc::new(CollectorRegistry::default());
    let _first = PrometheusServicesLogger::new(Arc::clone(&registry), None)
        .expect("the first logger builds");
    let second = PrometheusServicesLogger::new(Arc::clone(&registry), Some(&[2.5, 0.5]))
        .expect("the reused histograms never validate the new bucket list");
    assert!(second.payload_to_prometheus_map.contains_key("redis"));
}

#[test]
fn metric_factories_return_an_existing_registered_metric() {
    let registry = Arc::new(CollectorRegistry::default());
    let bounds = prepare_buckets(&[Bucket::Seconds(0.5)]).expect("the bounds are valid");
    registry
        .register(PrometheusMetric::new(
            CollectorKind::Histogram,
            "litellm_test_service_latency",
            "Latency for test_service service",
            &["test_service"],
            bounds,
        ))
        .expect("the histogram name is free");
    registry
        .register(PrometheusMetric::new(
            CollectorKind::Gauge,
            "litellm_test_service_size",
            "Gauge for test_service service",
            &["test_service"],
            Vec::new(),
        ))
        .expect("the gauge name is free");
    registry
        .register(PrometheusMetric::new(
            CollectorKind::Counter,
            "litellm_test_service_failed_requests",
            "Total failed_requests for test_service service",
            &["test_service", "error_class", "function_name"],
            Vec::new(),
        ))
        .expect("the counter name is free");

    let logger = PrometheusServicesLogger::new(Arc::clone(&registry), None)
        .expect("the service logger builds");

    let histogram = logger
        .create_histogram("test_service", "latency")
        .expect("the existing histogram comes back");
    assert!(Arc::ptr_eq(
        &histogram.samples,
        &registered(&registry, "litellm_test_service_latency").samples
    ));
    let gauge = logger
        .create_gauge("test_service", "size")
        .expect("the existing gauge comes back");
    assert!(Arc::ptr_eq(
        &gauge.samples,
        &registered(&registry, "litellm_test_service_size").samples
    ));
    let counter = logger
        .create_counter(
            "test_service",
            "failed_requests",
            &["error_class", "function_name"],
        )
        .expect("the existing counter comes back");
    assert!(Arc::ptr_eq(
        &counter.samples,
        &registered(&registry, "litellm_test_service_failed_requests").samples
    ));
}

#[test]
fn observe_histogram_records_under_the_service_label() {
    let (registry, _logger) = build(None);
    let histogram = registered(&registry, "litellm_redis_latency");
    PrometheusServicesLogger::observe_histogram(&histogram, "redis", 1.5);

    let recorded = sample(&histogram, &[Some("redis".to_owned())]);
    assert_eq!(recorded.count, 1);
    assert_eq!(
        recorded.total,
        MetricValue::Finite(Decimal::from_f64(1.5).expect("1.5 is exact"))
    );
    assert_eq!(recorded.buckets.get(7), Some(&0));
    assert_eq!(recorded.buckets.get(8), Some(&1));
    assert_eq!(recorded.buckets.last(), Some(&1));
}

#[test]
fn a_non_finite_observation_is_recorded_as_is() {
    let (registry, _logger) = build(None);
    let histogram = registered(&registry, "litellm_redis_latency");
    PrometheusServicesLogger::observe_histogram(&histogram, "redis", f64::NAN);

    let recorded = sample(&histogram, &[Some("redis".to_owned())]);
    assert_eq!(recorded.count, 1);
    assert_eq!(recorded.total, MetricValue::NotANumber);
    assert_eq!(recorded.buckets.iter().sum::<u64>(), 0);
}

#[test]
fn update_gauge_sets_the_value() {
    let (registry, _logger) = build(None);
    let gauge = registered(&registry, "litellm_pod_lock_manager_size");
    PrometheusServicesLogger::update_gauge(&gauge, "queue_a", 42.5);
    assert_eq!(
        sample(&gauge, &[Some("queue_a".to_owned())]).total,
        MetricValue::Finite(Decimal::from_f64(42.5).expect("42.5 is exact"))
    );
}

#[test]
fn gauge_values_keep_their_sign_and_finiteness() {
    let (registry, _logger) = build(None);
    let gauge = registered(&registry, "litellm_pod_lock_manager_size");
    PrometheusServicesLogger::update_gauge(&gauge, "positive", f64::INFINITY);
    PrometheusServicesLogger::update_gauge(&gauge, "negative", f64::NEG_INFINITY);
    PrometheusServicesLogger::update_gauge(&gauge, "not_a_number", f64::NAN);
    assert_eq!(
        sample(&gauge, &[Some("positive".to_owned())]).total,
        MetricValue::PositiveInfinity
    );
    assert_eq!(
        sample(&gauge, &[Some("negative".to_owned())]).total,
        MetricValue::NegativeInfinity
    );
    assert_eq!(
        sample(&gauge, &[Some("not_a_number".to_owned())]).total,
        MetricValue::NotANumber
    );
}

#[test]
fn increment_counter_adds_the_additional_labels() {
    let (registry, _logger) = build(None);
    let counter = registered(&registry, "litellm_redis_failed_requests");
    PrometheusServicesLogger::increment_counter(
        &counter,
        "redis",
        2.0,
        &["TimeoutError", "get_key_object"],
    );

    let recorded = sample(
        &counter,
        &[
            Some("redis".to_owned()),
            Some("TimeoutError".to_owned()),
            Some("get_key_object".to_owned()),
        ],
    );
    assert_eq!(recorded.total, MetricValue::Finite(Decimal::new(2, 0)));
    assert!(
        counter
            .series(&[Some("redis".to_owned()), None, None])
            .is_none()
    );
}

#[test]
fn increment_counter_without_additional_labels_uses_only_the_service() {
    let (registry, _logger) = build(None);
    let counter = registered(&registry, "litellm_redis_total_requests");
    PrometheusServicesLogger::increment_counter(&counter, "redis", 1.0, &[]);
    assert_eq!(
        sample(&counter, &[Some("redis".to_owned())]).total,
        MetricValue::Finite(Decimal::ONE)
    );
}

#[test]
fn service_success_hook_observes_latency_and_counts_requests() {
    let (registry, logger) = build(None);
    logger.service_success_hook(&success(ServiceType::Redis, 2.0));

    let histogram = registered(&registry, "litellm_redis_latency");
    assert_eq!(sample(&histogram, &[Some("redis".to_owned())]).count, 1);
    assert_eq!(
        sample(
            &registered(&registry, "litellm_redis_total_requests"),
            &[Some("redis".to_owned())]
        )
        .total,
        MetricValue::Finite(Decimal::ONE)
    );
    assert_eq!(
        series_count(&registered(&registry, "litellm_redis_failed_requests")),
        0
    );
}

#[test]
fn service_success_hook_leaves_a_gauge_service_alone() {
    let (registry, logger) = build(None);
    logger.service_success_hook(&success(ServiceType::PodLockManager, 1.0));
    assert_eq!(
        series_count(&registered(&registry, "litellm_pod_lock_manager_size")),
        0
    );
}

#[test]
fn service_failure_hook_increments_every_counter() {
    let (registry, logger) = build(None);
    logger.service_failure_hook(&failure(ServiceType::Redis, 0.5));

    assert_eq!(
        sample(
            &registered(&registry, "litellm_redis_total_requests"),
            &[Some("redis".to_owned())]
        )
        .total,
        MetricValue::Finite(Decimal::ONE)
    );
    // LiteLLM's sync failure hook passes no additional labels, so the failed-request labels stay unset
    assert_eq!(
        sample(
            &registered(&registry, "litellm_redis_failed_requests"),
            &[Some("redis".to_owned()), None, None]
        )
        .total,
        MetricValue::Finite(Decimal::ONE)
    );
    assert_eq!(
        series_count(&registered(&registry, "litellm_redis_latency")),
        0
    );
}

#[test]
fn async_service_success_hook_observes_and_counts() {
    let (registry, logger) = build(None);
    logger.async_service_success_hook(&success(ServiceType::Postgres, 2.0));

    let histogram = registered(&registry, "litellm_postgres_latency");
    let recorded = sample(&histogram, &[Some("postgres".to_owned())]);
    assert_eq!(recorded.count, 1);
    assert_eq!(recorded.buckets.get(8), Some(&1));
    assert_eq!(
        sample(
            &registered(&registry, "litellm_postgres_total_requests"),
            &[Some("postgres".to_owned())]
        )
        .total,
        MetricValue::Finite(Decimal::ONE)
    );
    assert_eq!(
        series_count(&registered(&registry, "litellm_postgres_failed_requests")),
        0
    );
}

#[test]
fn async_service_success_hook_updates_the_gauge_from_metadata() {
    let (registry, logger) = build(None);
    let mut event = success(ServiceType::PodLockManager, 0.0);
    event.event_metadata = Some(ServiceEventMetadata {
        gauge_labels: Some("queue_a".to_owned()),
        gauge_value: Some(3.0),
    });
    logger.async_service_success_hook(&event);

    assert_eq!(
        sample(
            &registered(&registry, "litellm_pod_lock_manager_size"),
            &[Some("queue_a".to_owned())]
        )
        .total,
        MetricValue::Finite(Decimal::new(3, 0))
    );
}

#[test]
fn async_service_success_hook_without_metadata_leaves_the_gauge_alone() {
    let (registry, logger) = build(None);
    logger.async_service_success_hook(&success(ServiceType::PodLockManager, 0.0));
    assert_eq!(
        series_count(&registered(&registry, "litellm_pod_lock_manager_size")),
        0
    );
}

#[test]
fn async_service_success_hook_defaults_missing_metadata_fields() {
    let (registry, logger) = build(None);
    let mut event = success(ServiceType::PodLockManager, 0.0);
    event.event_metadata = Some(ServiceEventMetadata::default());
    logger.async_service_success_hook(&event);

    assert_eq!(
        sample(
            &registered(&registry, "litellm_pod_lock_manager_size"),
            &[Some(String::new())]
        )
        .total,
        MetricValue::Finite(Decimal::ZERO)
    );
}

#[test]
fn async_service_failure_hook_labels_the_failed_counter() {
    let (registry, logger) = build(None);
    logger.async_service_failure_hook(
        &failure(ServiceType::Postgres, 0.25),
        "ConnectionRefusedError",
    );

    let failed = registered(&registry, "litellm_postgres_failed_requests");
    assert_eq!(
        sample(
            &failed,
            &[
                Some("postgres".to_owned()),
                Some("ConnectionRefusedError".to_owned()),
                Some("get_key_object".to_owned()),
            ]
        )
        .total,
        MetricValue::Finite(Decimal::ONE)
    );
    assert!(
        failed
            .series(&[Some("postgres".to_owned()), None, None])
            .is_none()
    );
    let total = registered(&registry, "litellm_postgres_total_requests");
    assert_eq!(
        sample(&total, &[Some("postgres".to_owned())]).total,
        MetricValue::Finite(Decimal::ONE)
    );
    assert_eq!(series_count(&total), 1);
    assert_eq!(
        series_count(&registered(&registry, "litellm_postgres_latency")),
        0
    );
}

#[test]
fn the_self_service_registers_and_emits_under_the_litellm_wire_name() {
    let (registry, logger) = build(None);
    assert!(registry.get("litellm_self_latency").is_some());
    assert!(registry.get("litellm_litellm_latency").is_none());

    logger.async_service_success_hook(&success(ServiceType::Litellm, 1.0));
    assert_eq!(
        sample(
            &registered(&registry, "litellm_self_total_requests"),
            &[Some("self".to_owned())]
        )
        .total,
        MetricValue::Finite(Decimal::ONE)
    );
}
