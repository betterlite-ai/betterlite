use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::fixtures::{details, guardrail_entry};
use super::support::Rig;
use crate::otel::legacy_lifecycle::tracer::DynamicOtel;
use crate::otel::legacy_policy::config::OpenTelemetryConfig;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::service::post_call::async_post_call_success_hook;

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

// _create_guardrail_span reads the dynamic params off the whole model_call_details
#[test]
fn otel_legacy_service_callbacks_behavior_tests_behavior() {
    let rig = Rig::new();
    let recorder = Arc::new(RecordingDynamic::default());
    let dynamic: Arc<dyn DynamicOtel> = recorder.clone();
    let lifecycle = rig.lifecycle_with_dynamic(dynamic);
    let mut call_details = details("call-y", Some(vec![guardrail_entry()]), None);
    let params = MetadataMap::from([(
        "team_id".to_owned(),
        OtelModelValue::String("team-dynamic".to_owned()),
    )]);
    call_details.insert(
        "standard_callback_dynamic_params".to_owned(),
        OtelModelValue::Object(params),
    );
    async_post_call_success_hook(&lifecycle, &call_details, None).expect("success hook");
    let seen = recorder.params_seen.lock().expect("recording lock");
    let recorded = seen.first().expect("the dynamic config hook was consulted");
    assert_eq!(
        recorded.get("team_id").and_then(OtelModelValue::as_str),
        Some("team-dynamic")
    );
    assert_eq!(rig.named("guardrail").len(), 1);
}
