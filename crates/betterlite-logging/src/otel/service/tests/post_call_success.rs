use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::fixtures::{details, guardrail_entry};
use super::support::{Rig, START, instant, text_attribute};
use crate::otel::legacy_lifecycle::tracer::DynamicOtel;
use crate::otel::legacy_policy::config::OpenTelemetryConfig;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::service::post_call::async_post_call_success_hook;

#[derive(Clone, Copy)]
enum Anchor {
    Live,
    Detached,
    NoGuardrails,
}

#[rstest::rstest]
#[case::live_parent(Anchor::Live)]
#[case::detached_parent(Anchor::Detached)]
#[case::no_guardrails(Anchor::NoGuardrails)]
fn otel_legacy_service_callbacks_behavior_tests_behavior(#[case] anchor: Anchor) {
    let rig = Rig::new();
    let lifecycle = rig.lifecycle();
    let span = match anchor {
        Anchor::Live | Anchor::NoGuardrails => rig.parent(&lifecycle, "proxy", START),
        Anchor::Detached => rig.detached(&lifecycle),
    };
    let entries = match anchor {
        Anchor::Live | Anchor::Detached => Some(vec![guardrail_entry()]),
        Anchor::NoGuardrails => None,
    };
    let details = details("call-s", entries, Some(&span));
    async_post_call_success_hook(&lifecycle, &details, Some(&span)).expect("success hook");
    let spans = rig.named("guardrail");
    match anchor {
        Anchor::NoGuardrails => assert!(spans.is_empty()),
        Anchor::Live => {
            assert_eq!(spans.len(), 1);
            assert_eq!(spans[0].parent_span_id, span.context.span_id());
            assert_eq!(spans[0].start_time, instant(1_704_067_200.5));
            assert_eq!(spans[0].end_time, instant(1_704_067_200.75));
            assert_eq!(text_attribute(&spans[0], "guardrail_name"), "pii");
            assert_eq!(
                text_attribute(&spans[0], "metadata.user_api_key_team_id"),
                "team-a"
            );
        }
        Anchor::Detached => {
            assert_eq!(spans.len(), 1, "a traceparent parent still gets the span");
            assert_eq!(spans[0].parent_span_id, span.context.span_id());
        }
    }
}

#[derive(Default)]
struct RecordingDynamic {
    params_seen: Mutex<Vec<MetadataMap>>,
}

impl DynamicOtel for RecordingDynamic {
    fn construct_dynamic_otel_headers(
        &self,
        _dynamic_params: &MetadataMap,
    ) -> Option<BTreeMap<String, String>> {
        None
    }

    fn construct_dynamic_otel_config(
        &self,
        dynamic_params: &MetadataMap,
    ) -> Option<OpenTelemetryConfig> {
        self.params_seen
            .lock()
            .expect("recording lock")
            .push(dynamic_params.clone());
        None
    }
}

// _create_guardrail_span picks the tracer from the whole model_call_details, dynamic params included
#[test]
fn otel_legacy_service_callbacks_behavior_tests_dynamic_params() {
    let rig = Rig::new();
    let recorder = Arc::new(RecordingDynamic::default());
    let dynamic: Arc<dyn DynamicOtel> = recorder.clone();
    let lifecycle = rig.lifecycle_with_dynamic(dynamic);
    let mut details = details("call-y", Some(vec![guardrail_entry()]), None);
    let params = MetadataMap::from([(
        "team_id".to_owned(),
        OtelModelValue::String("team-dynamic".to_owned()),
    )]);
    details.insert(
        "standard_callback_dynamic_params".to_owned(),
        OtelModelValue::Object(params.clone()),
    );
    async_post_call_success_hook(&lifecycle, &details, None).expect("success hook");
    let seen = recorder.params_seen.lock().expect("recording lock");
    let recorded = seen.first().expect("the dynamic config hook was consulted");
    assert_eq!(
        recorded.get("team_id").and_then(OtelModelValue::as_str),
        Some("team-dynamic")
    );
    assert_eq!(rig.named("guardrail").len(), 1);
}
