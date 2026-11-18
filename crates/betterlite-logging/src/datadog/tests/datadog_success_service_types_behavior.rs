use crate::datadog::service_events::DD_LOGGED_SUCCESS_SERVICE_TYPES;
use crate::logging_service_events::types::ServiceType;

#[test]
fn datadog_success_service_types_behavior() {
    // the budget reset job is the only service type Datadog hears a success record about
    assert_eq!(
        DD_LOGGED_SUCCESS_SERVICE_TYPES,
        [ServiceType::ResetBudgetJob].as_slice()
    );
    for service in ServiceType::ALL {
        assert_eq!(
            DD_LOGGED_SUCCESS_SERVICE_TYPES.contains(&service),
            service == ServiceType::ResetBudgetJob,
            "{service:?} is on the list only when it is the budget reset job"
        );
    }
}
