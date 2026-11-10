use betterlite_logging::spend_bodies::classifier::{
    SnapshotValue, classifier_audit_fields, classifier_input_snapshot, without_classifier_audit,
};
use betterlite_logging::spend_bodies::support::NodeAllocator;
use betterlite_logging::spend_sanitize::SpendSanitizePolicy;
use betterlite_logging::spend_sanitize::value::{SpendMapping, SpendValue, to_json};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::SnapshotSpec;
use super::pipeline::{Renderer, optional_decode, request_text, storage_policy_of};
use super::support::{decode, mapping_json, mapping_of};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClassifierSpec {
    store_bodies: bool,
    max_string_length: i64,
    turn_off_message_logging: bool,
    litellm_params: Value,
    standard_logging_object: Option<Value>,
    realtime_tools: Option<Value>,
    dynamic_params: Option<Value>,
    snapshots: Vec<SnapshotSpec>,
}

pub(super) fn classifier(spec: &ClassifierSpec) -> crate::helpers::HelperResult {
    let mut nodes = NodeAllocator::default();
    let mut render = Renderer {
        policy: storage_policy_of(
            SpendSanitizePolicy {
                store_bodies: spec.store_bodies,
                max_string_length: spec.max_string_length,
            },
            spec.turn_off_message_logging,
        ),
        nodes: &mut nodes,
    };
    let params = decode(&spec.litellm_params, render.nodes)?;
    let standard = optional_decode(spec.standard_logging_object.as_ref(), render.nodes)?;
    let tools = optional_decode(spec.realtime_tools.as_ref(), render.nodes)?;
    let dynamic = optional_decode(spec.dynamic_params.as_ref(), render.nodes)?;
    let snapshot_values = snapshots(&spec.snapshots, render.nodes)?;
    let standard_payload = standard.as_ref().and_then(|value| mapping_of(value).ok());
    let (audit, without) = audit_fields(standard_payload, render.nodes);
    let request = request_text(
        &mut render,
        &params,
        standard.as_ref(),
        tools.as_ref(),
        dynamic.as_ref(),
    )?;
    Ok(json!({
        "snapshots": snapshot_values,
        "audit": audit,
        "without": without,
        "request": request,
    }))
}

fn snapshots(specs: &[SnapshotSpec], nodes: &mut NodeAllocator) -> crate::helpers::HelperResult {
    let mut values = Vec::with_capacity(specs.len());
    for spec in specs {
        let value = decode(&spec.value, nodes)?;
        let snapshot =
            classifier_input_snapshot(SnapshotValue::Value(&value), spec.openai_sdk, nodes);
        values.push(snapshot.as_ref().map_or(Value::Null, |mapping| {
            mapping_json(mapping).unwrap_or(Value::Null)
        }));
    }
    Ok(Value::Array(values))
}

fn audit_fields(standard: Option<&SpendMapping>, nodes: &mut NodeAllocator) -> (Value, Value) {
    let Some(payload) = standard else {
        return (Value::Null, Value::Null);
    };
    let audit = json_object(&classifier_audit_fields(payload, nodes));
    let without = json_object(&without_classifier_audit(payload, nodes).entries);
    (audit, without)
}

fn json_object(entries: &IndexMap<String, SpendValue>) -> Value {
    let fields = entries
        .iter()
        .map(|(key, value)| (key.clone(), to_json(value)))
        .collect();
    serde_json::to_value(Json::Object(fields)).unwrap_or(Value::Null)
}
