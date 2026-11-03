#![cfg(test)]
use crate::otel::model::metadata::callback::auth_metadata;
use crate::otel::model::metadata::request_data::{
    metadata_from_request_data, model_from_request_data,
};
use crate::otel::model::utils::coercion::OtelModelValue;

fn input(source: &str) -> OtelModelValue {
    serde_json::from_str(source).expect("test JSON is valid")
}

#[test]
fn request_data_reads_model_and_requester_snapshot() {
    let data = input(
        r#"{"model":"group","metadata":{"requester_metadata":{"user_id":"u"},"ignored":"x"}}"#,
    );
    assert_eq!(model_from_request_data(&data).as_deref(), Some("group"));
    let snapshot = metadata_from_request_data(&data).expect("requester metadata exists");
    assert!(snapshot.contains_key("requester_metadata"));
    assert!(!snapshot.contains_key("ignored"));
}

#[test]
fn auth_metadata_keeps_only_string_values() {
    let payload = input(r#"{"metadata":{"user_api_key_auth_metadata":{"tenant":"t","count":2}}}"#);
    let kwargs = input("{}");
    assert_eq!(
        auth_metadata(Some(&payload), &kwargs)
            .and_then(|values| values.get("tenant").cloned())
            .as_deref(),
        Some("t")
    );
}
