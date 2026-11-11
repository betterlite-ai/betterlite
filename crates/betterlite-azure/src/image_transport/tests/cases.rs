#![cfg(test)]

use rstest::rstest;

use std::time::Duration;

use tokio::time::Instant;

use super::scenarios;
use crate::image_transport::{ImageClock as _, SystemClock};

#[rstest]
#[case::image_immediate_result(scenarios::image_immediate_result)]
#[case::operation_location_poll(scenarios::operation_location_poll)]
#[case::retry_after_poll(scenarios::retry_after_poll)]
#[case::poll_default_timeout_120(scenarios::poll_default_timeout_120)]
#[case::retry_after_default_10(scenarios::retry_after_default_10)]
#[case::same_origin_502(scenarios::same_origin_502)]
#[case::missing_operation_location_500(scenarios::missing_operation_location_500)]
#[case::missing_status_502(scenarios::missing_status_502)]
#[case::polling_timeout_408(scenarios::polling_timeout_408)]
#[case::retry_after_capped_at_deadline(scenarios::retry_after_capped_at_deadline)]
#[case::failed_operation_400(scenarios::failed_operation_400)]
#[case::retry_after_not_integer(scenarios::retry_after_not_integer)]
#[case::preview_submit_gate(scenarios::preview_submit_gate)]
#[case::deployment_body_model_drop(scenarios::deployment_body_model_drop)]
#[case::v1_body_deployment_model(scenarios::v1_body_deployment_model)]
#[case::provider_body_unchanged(scenarios::provider_body_unchanged)]
#[case::http_timeout_defaults(scenarios::http_timeout_defaults)]
fn image_transport_cases(#[case] scenario: fn()) {
    scenario();
}

#[tokio::test]
async fn system_clock_measures_and_pauses() {
    let clock = SystemClock {
        origin: Instant::now(),
    };
    let before = clock.now();
    clock.pause(1).await;
    assert!(clock.now().saturating_sub(before) >= Duration::from_secs(1));
    let negative = clock.now();
    clock.pause(-3).await;
    assert!(clock.now().saturating_sub(negative) < Duration::from_secs(1));
}
