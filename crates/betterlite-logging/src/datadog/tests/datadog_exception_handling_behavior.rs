use tracing::Level;

use crate::datadog::service_events::{async_service_failure_hook, async_service_success_hook};
use crate::logging_service_events::types::ServiceType;

use super::support::{Intake, build_logger, capture_logs, service_payload};

#[test]
fn datadog_exception_handling_behavior() {
    let (logger, transport) = build_logger(&[("DD_BATCH_SIZE", "1")], Intake::Accepted);
    let (_guard, logs) = capture_logs();

    // a duration JSON cannot carry fails the message, which the hook logs instead of propagating
    let failing_failure = service_payload(ServiceType::Redis, true, Some("boom"), f64::NAN);
    async_service_failure_hook(&logger, &failing_failure, None);
    assert_eq!(logs.count_containing(Level::ERROR, "service hook"), 1);
    assert_eq!(logger.queue.pending().len(), 0);
    assert_eq!(transport.post_count(), 0);

    let failing_success = service_payload(ServiceType::ResetBudgetJob, false, None, f64::NAN);
    async_service_success_hook(&logger, &failing_success, None);
    assert_eq!(logs.count_containing(Level::ERROR, "service hook"), 2);
    assert_eq!(logger.queue.pending().len(), 0);
    assert_eq!(transport.post_count(), 0);

    // the logger still works for the events after a failed one
    let successful = service_payload(ServiceType::ResetBudgetJob, false, None, 0.5);
    async_service_success_hook(&logger, &successful, None);
    assert_eq!(logger.queue.pending().len(), 1);
    assert_eq!(logs.count_containing(Level::ERROR, "service hook"), 2);
}
