use indexmap::IndexMap;

use crate::datadog::payloads::status::DataDogStatus;
use crate::datadog::proxy_failure::async_post_call_failure_hook;
use crate::datadog::service_events::{async_service_failure_hook, async_service_success_hook};
use crate::logging_service_events::types::ServiceType;

use super::support::{
    Intake, TestSpan, TestTracer, build_logger, exception, exception_logging, last_queued,
    normalizer, service_payload,
};

#[tokio::test]
async fn datadog_proxy_and_service_trace_context_behavior() {
    let span = TestSpan {
        trace_id: Some(12),
        span_id: Some(34),
    };
    let current = TestTracer(Some(&span), None);
    let root = TestTracer(None, Some(&span));
    let (logger, _) = build_logger(&[], Intake::Accepted);
    let patterns = normalizer();
    let facts = exception(r#"{"class_name": "APIError", "message": "failed"}"#);
    let context = exception_logging(Some(&facts), &patterns);

    // the proxy failure hook attaches the active span to the payload it queues
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
