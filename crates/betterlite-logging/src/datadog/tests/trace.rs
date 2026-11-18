use indexmap::IndexMap;

use crate::datadog::apm::{NullTracer, Tracer};
use crate::datadog::payloads::legacy::legacy_v1_logging_to_datadog;
use crate::datadog::payloads::standard::standard_logging_to_datadog;
use crate::datadog::payloads::status::DataDogStatus;
use crate::datadog::payloads::types::DatadogPayload;
use crate::datadog::proxy_failure::async_post_call_failure_hook;
use crate::datadog::service_events::{async_service_failure_hook, async_service_success_hook};
use crate::logging_service_events::types::ServiceType;

use super::support::{
    Intake, TestSpan, TestTracer, build_logger, default_environment, default_kwargs, exception,
    exception_logging, json, last_queued, legacy_event, normalizer, service_payload,
    standard_payload,
};

fn standard(tracer: &dyn Tracer) -> DatadogPayload {
    standard_logging_to_datadog(
        &standard_payload(),
        DataDogStatus::Info,
        &default_environment(),
        tracer,
    )
    .unwrap()
}

#[tokio::test]
async fn datadog_trace_context_attachment_behavior() {
    let span = TestSpan {
        trace_id: Some(12),
        span_id: Some(34),
    };
    let current = TestTracer(Some(&span), None);
    let built = standard(&current);
    assert_eq!(built.trace_id.as_deref(), Some("12"));
    assert_eq!(built.span_id.as_deref(), Some("34"));
    let kwargs = default_kwargs();
    let response = json(r#"{"usage": {}}"#);
    let legacy = legacy_v1_logging_to_datadog(
        &legacy_event(&kwargs, &response),
        &default_environment(),
        &current,
    )
    .unwrap();
    assert_eq!(legacy.trace_id.as_deref(), Some("12"));
    assert_eq!(legacy.span_id.as_deref(), Some("34"));
    let root = TestTracer(None, Some(&span));
    assert_eq!(standard(&root).trace_id.as_deref(), Some("12"));
    let trace_only = TestSpan {
        trace_id: Some(7),
        span_id: None,
    };
    let partial = standard(&TestTracer(Some(&trace_only), None));
    assert_eq!(partial.trace_id.as_deref(), Some("7"));
    assert_eq!(partial.span_id, None);
    let id_only = TestSpan {
        trace_id: None,
        span_id: Some(9),
    };
    let skipped = standard(&TestTracer(Some(&id_only), None));
    assert_eq!(skipped.trace_id, None);
    assert_eq!(skipped.span_id, None);
    let untraced = standard(&NullTracer);
    assert_eq!(untraced.trace_id, None);
    assert_eq!(untraced.span_id, None);

    // the proxy failure hook attaches the active span to the payload it queues
    let (logger, _) = build_logger(&[], Intake::Accepted);
    let normalizer = normalizer();
    let facts = exception(r#"{"class_name": "APIError", "message": "failed"}"#);
    let context = exception_logging(Some(&facts), &normalizer);
    async_post_call_failure_hook(&logger, &current, &context, None, IndexMap::new()).await;
    let failed = last_queued(&logger);
    assert_eq!(failed.trace_id.as_deref(), Some("12"));
    assert_eq!(failed.span_id.as_deref(), Some("34"));
    async_post_call_failure_hook(&logger, &root, &context, None, IndexMap::new()).await;
    assert_eq!(last_queued(&logger).trace_id.as_deref(), Some("12"));

    // the service hooks queue their payloads without any span context, even from the same tracer
    async_service_failure_hook(
        &logger,
        &service_payload(ServiceType::Redis, true, Some("boom"), 0.1),
        None,
    );
    let service_failed = last_queued(&logger);
    assert_eq!(service_failed.status, DataDogStatus::Warn);
    assert_eq!(service_failed.trace_id, None);
    assert_eq!(service_failed.span_id, None);
    async_service_success_hook(
        &logger,
        &service_payload(ServiceType::ResetBudgetJob, false, None, 0.1),
        None,
    );
    let service_succeeded = last_queued(&logger);
    assert_eq!(service_succeeded.status, DataDogStatus::Info);
    assert_eq!(service_succeeded.trace_id, None);
    assert_eq!(service_succeeded.span_id, None);
}
