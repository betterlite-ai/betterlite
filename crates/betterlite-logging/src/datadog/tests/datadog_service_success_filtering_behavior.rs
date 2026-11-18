use betterlite_models::decisions::Json;

use crate::datadog::payloads::status::DataDogStatus;
use crate::datadog::service_events::async_service_success_hook;
use crate::logging_service_events::types::ServiceType;

use super::support::{
    Intake, build_logger, field, json, last_queued, message, service_payload, text,
};

#[test]
fn datadog_service_success_filtering_behavior() {
    let (logger, transport) = build_logger(&[("DD_BATCH_SIZE", "1")], Intake::Accepted);
    let payload = service_payload(ServiceType::ResetBudgetJob, false, None, 0.5);
    async_service_success_hook(&logger, &payload, None);

    // the one listed service type logs its success, without flushing even at the batch size
    assert_eq!(logger.queue.pending().len(), 1);
    assert_eq!(transport.post_count(), 0);
    let queued = last_queued(&logger);
    assert_eq!(queued.status, DataDogStatus::Info);
    assert_eq!(queued.trace_id, None);
    assert_eq!(queued.span_id, None);
    let logged = message(&queued);
    assert_eq!(field(&logged, "is_error"), &Json::Bool(false));
    assert_eq!(field(&logged, "error"), &Json::Null);
    assert_eq!(text(field(&logged, "service")), "reset_budget_job");
    assert_eq!(field(&logged, "duration"), &json("0.5"));

    // every other service type returns silently
    for service in ServiceType::ALL {
        if service == ServiceType::ResetBudgetJob {
            continue;
        }
        async_service_success_hook(&logger, &service_payload(service, false, None, 0.5), None);
        assert_eq!(logger.queue.pending().len(), 1);
    }
    assert_eq!(transport.post_count(), 0);
}
