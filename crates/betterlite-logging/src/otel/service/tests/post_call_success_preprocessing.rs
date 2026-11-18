use opentelemetry::trace::Span as _;

use super::fixtures::{details, number};
use super::support::{Rig, START, attribute, f64_attribute};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::service::post_call::async_post_call_success_hook;
use crate::otel::vocabulary::constants::attribute_keys::litellm;

fn with_anchors(
    mut details: MetadataMap,
    received: Option<f64>,
    first: Option<f64>,
) -> MetadataMap {
    if let Some(value) = received {
        let Some(OtelModelValue::Object(params)) = details.get_mut("litellm_params") else {
            panic!("litellm_params must be an object");
        };
        let Some(OtelModelValue::Object(metadata)) = params.get_mut("metadata") else {
            panic!("litellm_params.metadata must be an object");
        };
        metadata.insert("litellm_received_at".to_owned(), number(value));
    }
    if let Some(value) = first {
        details.insert("first_api_call_start_time".to_owned(), number(value));
    }
    details
}

#[rstest::rstest]
#[case::anchors_measure(Some(1_704_067_200.0), Some(1_704_067_201.5), Some(1500.0))]
#[case::clock_skew_is_omitted(Some(1_704_067_202.0), Some(1_704_067_201.5), None)]
#[case::missing_received_at(None, Some(1_704_067_201.5), None)]
#[case::missing_first_handoff(Some(1_704_067_200.0), None, None)]
fn otel_legacy_service_callbacks_behavior_tests_behavior(
    #[case] received: Option<f64>,
    #[case] first_handoff: Option<f64>,
    #[case] expected: Option<f64>,
) {
    let rig = Rig::new();
    let lifecycle = rig.lifecycle();
    let parent = rig.parent(&lifecycle, "proxy", START);
    let details = with_anchors(
        details("call-sp", None, Some(&parent)),
        received,
        first_handoff,
    );
    async_post_call_success_hook(&lifecycle, &details, Some(&parent)).expect("success hook");
    parent.lock().end();
    let server = rig.span("proxy");
    match expected {
        Some(ms) => assert_eq!(f64_attribute(&server, litellm::PREPROCESSING_MS), ms),
        None => assert!(attribute(&server, litellm::PREPROCESSING_MS).is_none()),
    }
}
