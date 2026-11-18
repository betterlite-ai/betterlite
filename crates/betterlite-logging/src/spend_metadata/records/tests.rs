#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;
use serde_json::Map;
use serde_json::Value;

use super::{SpendLogsMetadata, SpendLogsRouterMetadata};

fn serialized(record: &SpendLogsMetadata) -> Map<String, Value> {
    serde_json::to_value(record)
        .expect("the record serializes")
        .as_object()
        .cloned()
        .expect("the record serializes as an object")
}

#[rstest]
#[case::actor_agent_id("actor_agent_id")]
#[case::target_agent_id("target_agent_id")]
#[case::billing_agent_id("billing_agent_id")]
#[case::agent_execution_mode("agent_execution_mode")]
#[case::verified_human_user_id("verified_human_user_id")]
#[case::routing_decision("routing_decision")]
fn the_absent_constructor_omits_the_agent_fields_and_routing_decision(#[case] absent: &str) {
    let fields = serialized(&SpendLogsMetadata::default());
    assert_eq!(fields.len(), 42);
    assert!(!fields.contains_key(absent), "{absent} is absent");
    assert_eq!(fields.get("user_api_key"), Some(&Value::Null));
    assert_eq!(fields.get("status"), Some(&Value::Null));
}

#[test]
fn a_present_agent_field_serializes_its_null_value() {
    let mut record = SpendLogsMetadata::default();
    record.actor_agent_id = Some(None);
    let fields = serialized(&record);
    assert_eq!(fields.len(), 43);
    assert_eq!(fields.get("actor_agent_id"), Some(&Value::Null));
}

#[test]
fn routing_decision_serializes_its_payload() {
    let mut record = SpendLogsMetadata::default();
    record.routing_decision = Some(Some(Json::Text("router".to_owned())));
    let fields = serialized(&record);
    assert_eq!(
        fields.get("routing_decision"),
        Some(&Value::String("router".to_owned()))
    );
}

#[rstest]
#[case::requested_model("requested_model")]
#[case::selected_model("selected_model")]
#[case::selected_provider("selected_provider")]
#[case::router_correlation_id("router_correlation_id")]
fn the_router_metadata_serializes_every_field_as_null(#[case] name: &str) {
    let fields = serde_json::to_value(SpendLogsRouterMetadata::default())
        .expect("the router metadata serializes")
        .as_object()
        .cloned()
        .expect("the router metadata serializes as an object");
    assert_eq!(fields.len(), 4);
    assert_eq!(fields.get(name), Some(&Value::Null), "{name} is null");
}

#[test]
fn decoding_distinguishes_missing_from_null() {
    let record: SpendLogsMetadata = serde_json::from_str(
        r#"{"actor_agent_id": "actor", "target_agent_id": null, "status": "failure", "unknown": true}"#,
    )
    .expect("the record decodes");
    assert_eq!(record.actor_agent_id, Some(Some("actor".to_owned())));
    assert_eq!(record.target_agent_id, Some(None));
    assert_eq!(record.billing_agent_id, None);
    assert_eq!(record.status.as_deref(), Some("failure"));
    assert_eq!(record.routing_decision, None);
}

#[test]
fn a_mistyped_field_fails_to_decode() {
    let error = serde_json::from_str::<SpendLogsMetadata>(r#"{"status": 5}"#)
        .expect_err("the status is not text");
    assert!(error.to_string().contains("invalid type"));
}
