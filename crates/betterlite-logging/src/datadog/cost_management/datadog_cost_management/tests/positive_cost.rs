use jiff::tz::TimeZone;

use super::support::{self, Fixture};
use crate::datadog::cost_management::datadog_cost_management::aggregate;

#[tokio::test]
async fn positive_cost_records_are_queued() {
    let fixture = Fixture::new(&[]);
    let logger = fixture.build();
    assert!(logger.log_success(support::default_payload().build()).await);
    assert_eq!(logger.queue_length(), 1);
}

#[tokio::test]
async fn positive_cost_zero_and_negative_records_are_dropped() {
    let fixture = Fixture::new(&[]);
    let logger = fixture.build();
    let zero = support::default_payload()
        .set("/response_cost", support::number(0.0))
        .build();
    let negative = support::default_payload()
        .set("/response_cost", support::number(-0.5))
        .build();
    assert!(!logger.log_success(zero).await);
    assert!(!logger.log_success(negative).await);
    assert_eq!(logger.queue_length(), 0);
}

#[tokio::test]
async fn positive_cost_zero_records_are_not_aggregated() {
    let fixture = Fixture::new(&[]);
    let logger = fixture.build();
    let zero = support::default_payload()
        .set("/response_cost", support::number(0.0))
        .build();
    let entries = aggregate::aggregate(&[zero], &logger.settings, &TimeZone::UTC);
    assert!(entries.is_empty());
}
