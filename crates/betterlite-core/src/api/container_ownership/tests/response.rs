use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::super::response::{
    dump_response, owner_file_object, response_provider, synthetic_container,
};
use super::super::{ContainerIdentity, DEFAULT_PROVIDER};
use crate::api::response_id_traversal::record::Record;

#[test]
fn dump_keeps_every_key_of_a_mapping() {
    let response: Record = serde_json::from_str(
        r#"{"id":"container-1","object":"container","model":null,"_hidden_params":{"custom_llm_provider":"azure"}}"#,
    )
    .unwrap();
    let dumped = dump_response(&response);
    assert_eq!(
        dumped.get("id"),
        Some(&Json::Text("container-1".to_owned()))
    );
    assert_eq!(dumped.get("model"), Some(&Json::Null));
    assert_eq!(
        dumped.get("object"),
        Some(&Json::Text("container".to_owned()))
    );
    assert_eq!(
        dumped.get("_hidden_params"),
        Some(&Json::Object(IndexMap::from([(
            "custom_llm_provider".to_owned(),
            Json::Text("azure".to_owned())
        )])))
    );
}

#[test]
fn dump_drops_hidden_params_of_a_typed_object() {
    let response: Record = serde_json::from_str(
        r#"{"__object__":{"fields":{"id":"container-1","_hidden_params":{"custom_llm_provider":"azure"}}}}"#,
    )
    .unwrap();
    let dumped = dump_response(&response);
    assert_eq!(
        dumped.get("id"),
        Some(&Json::Text("container-1".to_owned()))
    );
    assert!(!dumped.contains_key("_hidden_params"));
}

#[test]
fn dump_of_a_non_mapping_keeps_only_a_null_id() {
    let response = Record::Scalar(Json::Text("container-1".to_owned()));
    assert_eq!(
        dump_response(&response),
        IndexMap::from([("id".to_owned(), Json::Null)])
    );
}

#[test]
fn owner_file_object_stamps_the_provider_and_upstream_id() {
    let response: Record =
        serde_json::from_str(r#"{"id":"container-1","custom_llm_provider":"stale"}"#).unwrap();
    let identity = ContainerIdentity {
        original_container_id: "container-1".to_owned(),
        custom_llm_provider: "azure".to_owned(),
    };
    let Json::Object(fields) = owner_file_object(&response, &identity) else {
        panic!("the owner file object is a mapping");
    };
    assert_eq!(
        fields.get("custom_llm_provider"),
        Some(&Json::Text("azure".to_owned()))
    );
    assert_eq!(
        fields.get("provider_container_id"),
        Some(&Json::Text("container-1".to_owned()))
    );
    assert_eq!(
        fields.get("id"),
        Some(&Json::Text("container-1".to_owned()))
    );
}

#[test]
fn response_provider_reads_the_hidden_params() {
    let response: Record =
        serde_json::from_str(r#"{"_hidden_params":{"custom_llm_provider":"azure"}}"#).unwrap();
    assert_eq!(response_provider(&response), "azure");
}

#[test]
fn response_provider_defaults_to_openai() {
    let absent: Record = serde_json::from_str(r#"{"id":"container-1"}"#).unwrap();
    assert_eq!(response_provider(&absent), DEFAULT_PROVIDER);
    let empty: Record =
        serde_json::from_str(r#"{"_hidden_params":{"custom_llm_provider":""}}"#).unwrap();
    assert_eq!(response_provider(&empty), DEFAULT_PROVIDER);
    let non_mapping: Record = serde_json::from_str(r#"{"_hidden_params":["azure"]}"#).unwrap();
    assert_eq!(response_provider(&non_mapping), DEFAULT_PROVIDER);
}

#[test]
fn response_provider_reads_a_typed_object() {
    let response: Record = serde_json::from_str(
        r#"{"__object__":{"fields":{"_hidden_params":{"custom_llm_provider":"azure"}}}}"#,
    )
    .unwrap();
    assert_eq!(response_provider(&response), "azure");
}

#[test]
fn synthetic_container_carries_an_id_and_an_object() {
    let expected: Record =
        serde_json::from_str(r#"{"id":"container-1","object":"container"}"#).unwrap();
    assert_eq!(synthetic_container("container-1"), expected);
}
