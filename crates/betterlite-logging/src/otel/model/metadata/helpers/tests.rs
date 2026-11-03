#![cfg(test)]
use crate::otel::model::metadata::identity::{flatten_metadata, resolve_provider_model};
use crate::otel::model::utils::coercion::OtelModelValue;

#[test]
fn provider_model_uses_hidden_model_before_call_model() {
    let payload: OtelModelValue =
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
