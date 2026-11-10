use betterlite_models::decisions::Json;
use betterlite_vertex::provider_metadata::redaction::{
    redact_litellm_params_metadata, redact_logged_object_metadata,
};
use betterlite_vertex::request_labels::{pop_vertex_request_labels, request_labels_from_params};
use serde_json::{Value, json};

use super::super::input::LabelCase;

pub(super) fn labels(cases: &[LabelCase]) -> Value {
    let entries: Vec<Value> = cases.iter().map(label_value).collect();
    json!({
        "cases": entries,
    })
}

fn label_value(case: &LabelCase) -> Value {
    let mut optional_params = case.optional_params.clone();
    let labels = pop_vertex_request_labels(&mut optional_params, case.litellm_params.as_ref());
    let from_params = request_labels_from_params(case.litellm_params.as_ref());
    json!({
        "labels": labels,
        "from_params": from_params,
        "optional_params": optional_params,
    })
}

pub(super) fn metadata(logged_objects: &[Json], call_details: &[Json]) -> Value {
    let mut logged = logged_objects.to_vec();
    for object in &mut logged {
        redact_logged_object_metadata(object);
    }
    let mut details = call_details.to_vec();
    for detail in &mut details {
        redact_litellm_params_metadata(detail);
    }
    json!({"logged_objects": logged, "call_details": details})
}
