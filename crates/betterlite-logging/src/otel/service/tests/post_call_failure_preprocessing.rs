use super::fixtures::{error_config, number, request_data};
use super::support::{Rig, START, attribute, f64_attribute};
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::service::post_call::{
    PostCallFailure, ProxyCallKey, async_post_call_failure_hook,
};
use crate::otel::vocabulary::constants::attribute_keys::litellm;

#[rstest::rstest]
#[case::anchors_measure(
    Some(1_704_067_200.0_f64),
    Some(1_704_067_201.5_f64),
    Some(1_500.0_f64)
)]
#[case::clock_skew_is_omitted(Some(1_704_067_202.0_f64), Some(1_704_067_201.5_f64), None)]
#[case::missing_received_at(None, Some(1_704_067_201.5_f64), None)]
#[case::missing_first_handoff(Some(1_704_067_200.0_f64), None, None)]
fn otel_legacy_service_callbacks_behavior_tests_behavior(
    #[case] received: Option<f64>,
    #[case] first_handoff: Option<f64>,
    #[case] expected: Option<f64>,
) {
    let rig = Rig::new();
    let lifecycle = rig.lifecycle();
    let parent = rig.parent(&lifecycle, "proxy", START);
    let mut data = request_data("call-p", vec![]);
    if let Some(value) = received {
        let Some(OtelModelValue::Object(metadata)) = data.get_mut("metadata") else {
            panic!("request_data.metadata must be an object");
        };
        metadata.insert("litellm_received_at".to_owned(), number(value));
    }
    if let Some(value) = first_handoff {
        data.insert("first_api_call_start_time".to_owned(), number(value));
    }
    let key = ProxyCallKey {
        parent_span: Some(&parent),
        team_id: None,
        team_alias: None,
    };
    let config = error_config();
    let call = PostCallFailure {
        request_data: &data,
        exception: None,
        key: &key,
        traceback: None,
        info: &config,
    };
    async_post_call_failure_hook(&lifecycle, &call).expect("failure hook");
    let server = rig.span("proxy");
    match expected {
        Some(ms) => assert_eq!(
            f64_attribute(&server, litellm::PREPROCESSING_MS).to_bits(),
            ms.to_bits()
        ),
        None => assert!(attribute(&server, litellm::PREPROCESSING_MS).is_none()),
    }
}
