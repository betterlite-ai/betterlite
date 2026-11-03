#![cfg(test)]
use crate::otel::model::metadata::extraction::auth_metadata;
use crate::otel::model::metadata::extraction::metadata_from_request_data;
use crate::otel::model::metadata::wire::{callback::RequestData, decode};
use serde::de::DeserializeOwned;

fn input<T>(source: &str) -> T
where
    T: DeserializeOwned,
{
    decode(source).expect("test JSON is valid")
}

#[test]
fn request_data_reads_model_and_requester_snapshot() {
    let data: RequestData = input(
        r#"{"model":"group","metadata":{"requester_metadata":{"user_id":"u"},"ignored":"x"}}"#,
    );
    assert_eq!(data.model.as_deref(), Some("group"));
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

#[rstest::rstest]
#[case(
    r#"{"metadata":{"user_api_key_auth_metadata":{"tenant":"closed"}}}"#,
    "closed"
)]
#[case(
    r#"{"metadata":{"user_api_key_auth_metadata":{"number":1}}}"#,
    "primary"
)]
fn auth_config_falls_back_in_payload_then_metadata_order(
    #[case] source: &str,
    #[case] expected: &str,
) {
    let kwargs = input(
        r#"{"litellm_params":{"metadata":{"user_api_key_auth_metadata":{"tenant":"primary"}},"litellm_metadata":{"user_api_key_auth_metadata":{"tenant":"secondary"}}}}"#,
    );
    let payload = input(source);
    let config = auth_metadata(Some(&payload), &kwargs).unwrap();
    assert_eq!(config.get("tenant").map(String::as_str), Some(expected));
    assert_eq!(config.len(), 1);
}

#[test]
fn anthropic_metadata_is_read_without_primary_metadata() {
    let secondary = input(
        r#"{"litellm_params":{"litellm_metadata":{"user_api_key_auth_metadata":{"tenant":"secondary"}}}}"#,
    );
    assert_eq!(
        auth_metadata(None, &secondary)
            .unwrap()
            .get("tenant")
            .map(String::as_str),
        Some("secondary")
    );
    let snapshot = input(
        r#"{"litellm_metadata":{"requester_metadata":{"project":"caller"},"user_api_key_user_id":"server"}}"#,
    );
    assert_eq!(metadata_from_request_data(&snapshot).unwrap().len(), 1);
}
