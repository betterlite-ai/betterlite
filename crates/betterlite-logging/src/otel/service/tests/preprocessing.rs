use opentelemetry::trace::Span as _;

use super::support::{Rig, START, attribute, f64_attribute, kwargs};
use crate::otel::service::preprocessing::set_preprocessing_duration_attribute;
use crate::otel::vocabulary::constants::attribute_keys::litellm;

#[rstest::rstest]
#[case::litellm_params_metadata(
    r#"{"litellm_params":{"metadata":{"litellm_received_at":1704067200.0}},"first_api_call_start_time":1704067201.5}"#,
    Some(1500.0)
)]
#[case::top_level_metadata(
    r#"{"metadata":{"litellm_received_at":1704067200.0},"first_api_call_start_time":1704067201.5}"#,
    Some(1500.0)
)]
#[case::litellm_metadata(
    r#"{"litellm_metadata":{"litellm_received_at":1704067200.0},"first_api_call_start_time":1704067201.5}"#,
    Some(1500.0)
)]
#[case::params_bucket_wins(
    r#"{"litellm_params":{"metadata":{"litellm_received_at":1704067200.0}},"metadata":{"litellm_received_at":1704067300.0},"first_api_call_start_time":1704067201.5}"#,
    Some(1500.0)
)]
#[case::string_with_fraction(
    r#"{"metadata":{"litellm_received_at":"2024-01-01 00:00:00.500000"},"first_api_call_start_time":1704067201.5}"#,
    Some(1000.0)
)]
#[case::string_without_fraction(
    r#"{"metadata":{"litellm_received_at":"2024-01-01 00:00:00"},"first_api_call_start_time":1704067201.5}"#,
    Some(1500.0)
)]
#[case::unparsable_string(
    r#"{"metadata":{"litellm_received_at":"midnight"},"first_api_call_start_time":1704067201.5}"#,
    None
)]
#[case::first_handoff_must_be_top_level(
    r#"{"metadata":{"litellm_received_at":1704067200.0},"litellm_params":{"first_api_call_start_time":1704067201.5}}"#,
    None
)]
#[case::zero_delay_is_kept(
    r#"{"metadata":{"litellm_received_at":1704067200.0},"first_api_call_start_time":1704067200.0}"#,
    Some(0.0)
)]
fn otel_legacy_service_callbacks_behavior_tests_behavior(
    #[case] container: &str,
    #[case] expected: Option<f64>,
) {
    let rig = Rig::new();
    let lifecycle = rig.lifecycle();
    let parent = rig.parent(&lifecycle, "proxy", START);
    let container = kwargs(container);
    set_preprocessing_duration_attribute(Some(&parent), &container);
    parent.lock().end();
    let span = rig.span("proxy");
    match expected {
        Some(ms) => assert_eq!(f64_attribute(&span, litellm::PREPROCESSING_MS), ms),
        None => assert!(attribute(&span, litellm::PREPROCESSING_MS).is_none()),
    }
}
