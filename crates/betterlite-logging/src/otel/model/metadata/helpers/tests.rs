#![cfg(test)]
use crate::otel::model::metadata::helpers::{flatten_metadata, resolve_provider_model};
use crate::otel::model::metadata::request_identity::parse::from_payload;
use crate::otel::model::metadata::wire::payload::Payload;
use crate::otel::model::utils::coercion::OtelModelValue;

#[test]
fn provider_model_uses_hidden_model_before_call_model() {
    let payload: Payload =
        serde_json::from_str(r#"{"model":"call","hidden_params":{"litellm_model_name":"hidden"}}"#)
            .expect("test JSON is valid");
    assert_eq!(resolve_provider_model(&payload).as_deref(), Some("hidden"));
}

#[test]
fn flatten_metadata_uses_dotted_scalar_paths() {
    let metadata: OtelModelValue =
        serde_json::from_str(r#"{"outer":{"inner":"value"}}"#).expect("test JSON is valid");
    let fields = match &metadata {
        OtelModelValue::Object(fields) => Some(fields),
        OtelModelValue::Null(())
        | OtelModelValue::Bool(_)
        | OtelModelValue::Number(_)
        | OtelModelValue::String(_)
        | OtelModelValue::Array(_) => None,
    };
    let flattened = flatten_metadata(fields);
    assert_eq!(
        flattened.get("outer.inner").map(String::as_str),
        Some("value")
    );
}

#[rstest::rstest]
#[case("{}")]
#[case(r#"{"metadata":{"user_api_key_team_metadata":{}}}"#)]
fn empty_team_metadata_is_absent(#[case] source: &str) {
    let payload: Payload = serde_json::from_str(source).unwrap();
    let identity = from_payload(&payload).unwrap();
    assert!(identity.team_metadata.is_none());
    assert_eq!(resolve_provider_model(&payload), None);
}

#[test]
fn team_metadata_stays_raw_until_promotion() {
    let payload: Payload = serde_json::from_str(
        r#"{"metadata":{"user_api_key_team_metadata":{"private":"retained","nested":{"key":1}}}}"#,
    )
    .unwrap();
    let identity = from_payload(&payload).unwrap();
    let metadata = identity.team_metadata.unwrap();
    assert_eq!(
        metadata.get("private").and_then(OtelModelValue::as_str),
        Some("retained")
    );
    assert!(
        metadata
            .get("nested")
            .and_then(OtelModelValue::as_object)
            .is_some()
    );
}
