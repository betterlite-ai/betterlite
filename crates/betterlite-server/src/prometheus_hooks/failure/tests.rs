#![cfg(test)]

use std::sync::Arc;

use betterlite_logging::callback_logger::CallbackLogger as _;
use betterlite_logging::prometheus::budget_gauges::lists::BudgetTableSnapshot;
use betterlite_logging::prometheus::budget_lookups::records::OrgRecord;
use betterlite_logging::prometheus_helpers::bounded_series_tracker::SeriesLimits;
use rust_decimal::Decimal;

use super::super::tests::support::{
    DEPLOYMENT_FAILURES, FAILED, ORG_BUDGET, dec, error_information, event, fixture,
    fixture_with_limits, payload, totals,
};

#[tokio::test]
async fn invalid_key_failure_is_skipped() {
    let fixture = fixture();
    let mut record = payload(Decimal::ZERO);
    record.error_information = Some(error_information("invalid api key"));
    fixture
        .hooks
        .async_log_failure_event(Arc::new(event(record)))
        .await
        .unwrap();
    assert!(totals(&fixture.registry, FAILED).is_empty());
    assert!(totals(&fixture.registry, DEPLOYMENT_FAILURES).is_empty());
    assert!(fixture.lookups.orgs().is_empty());
}

#[tokio::test]
async fn failed_counter_deployment_and_org_budget_run() {
    let fixture = fixture();
    fixture.lookups.set_org_record(OrgRecord {
        spend: Some(dec("10")),
        organization_alias: Some("org-alias".to_owned()),
        litellm_budget_table: Some(BudgetTableSnapshot {
            max_budget: Some(dec("100")),
            budget_reset_at: None,
        }),
    });
    let mut record = payload(dec("15"));
    record.metadata.user_api_key_org_id = Some("org-1".to_owned());
    record.error_information = Some(error_information("Something went wrong"));
    fixture
        .hooks
        .async_log_failure_event(Arc::new(event(record)))
        .await
        .unwrap();
    assert_eq!(totals(&fixture.registry, FAILED), vec![Decimal::ONE]);
    assert_eq!(
        totals(&fixture.registry, DEPLOYMENT_FAILURES),
        vec![Decimal::ONE]
    );
    assert_eq!(fixture.lookups.orgs(), vec!["org-1".to_owned()]);
    // the failure event asks for the org gauge with a zero cost, so the stored spend alone comes off the budget
    assert_eq!(totals(&fixture.registry, ORG_BUDGET), vec![dec("90")]);
}

#[tokio::test]
async fn errors_inside_the_failure_event_are_swallowed() {
    let fixture = fixture_with_limits(SeriesLimits {
        max_series: Some(0),
        ttl: None,
        cleanup_interval: None,
    });
    let mut record = payload(Decimal::ZERO);
    record.error_information = Some(error_information("Something went wrong"));
    fixture
        .hooks
        .async_log_failure_event(Arc::new(event(record)))
        .await
        .unwrap();
    assert!(totals(&fixture.registry, FAILED).is_empty());
}
