use indexmap::IndexMap;

use super::support::mapping;
use crate::python_json;
use crate::spend_bodies::placeholder::{placeholder_request_body, placeholder_request_metadata};
use crate::spend_bodies::support::NodeAllocator;
use crate::spend_sanitize::value::{NodeId, SpendList, SpendMapping, SpendValue, to_json};

fn stored(params: &SpendMapping, model_group: &str, raw_model: &str) -> String {
    let placeholdered = placeholder_request_body(
        params,
        model_group,
        raw_model,
        &mut NodeAllocator::default(),
    );
    python_json::dumps(&to_json(&SpendValue::Mapping(placeholdered)))
}

#[test]
fn placeholders_an_existing_model_and_existing_metadata_fields() {
    let params = mapping(
        r#"{"proxy_server_request": {"body": {"model": "unsafe-name", "metadata": {"model_group": "g", "error_information": {"error_message": "model gpt was bad", "traceback": "gpt failed", "error_class": "x"}, "other": 1}}}}"#,
    );
    assert_eq!(
        stored(&params, "group", "gpt"),
        r#"{"proxy_server_request": {"body": {"model": "unknown-model", "metadata": {"model_group": "group", "error_information": {"error_message": "model unknown-model was bad", "traceback": "unknown-model failed", "error_class": "x"}, "other": 1}}}}"#
    );
}

#[test]
fn does_not_insert_a_missing_model_or_model_group() {
    let params = mapping(r#"{"proxy_server_request": {"body": {"metadata": {"other": 1}}}}"#);
    assert_eq!(
        stored(&params, "group", "gpt"),
        r#"{"proxy_server_request": {"body": {"metadata": {"other": 1}}}}"#
    );
}

#[test]
fn keeps_a_non_mapping_error_information() {
    let params = mapping(
        r#"{"proxy_server_request": {"body": {"model": "m", "metadata": {"model_group": "g", "error_information": "plain text"}}}}"#,
    );
    assert_eq!(
        stored(&params, "group", "gpt"),
        r#"{"proxy_server_request": {"body": {"model": "unknown-model", "metadata": {"model_group": "group", "error_information": "plain text"}}}}"#
    );
}

#[rstest::rstest]
#[case::missing_proxy(r#"{"model": "m"}"#)]
#[case::null_proxy(r#"{"proxy_server_request": null}"#)]
#[case::missing_body(r#"{"proxy_server_request": {"headers": {}}}"#)]
fn leaves_params_without_a_stored_body_alone(#[case] source: &str) {
    let params = mapping(source);
    assert_eq!(
        stored(&params, "group", "gpt"),
        python_json::dumps(&to_json(&SpendValue::Mapping(params)))
    );
}

#[test]
fn passes_foreign_objects_through() {
    let model = SpendValue::Model(SpendMapping {
        node: NodeId::from(1_u64),
        entries: IndexMap::from([("content".to_owned(), SpendValue::Text("foreign".to_owned()))]),
    });
    let body = SpendValue::Mapping(SpendMapping {
        node: NodeId::from(2_u64),
        entries: IndexMap::from([
            ("model".to_owned(), SpendValue::Text("m".to_owned())),
            (
                "messages".to_owned(),
                SpendValue::List(SpendList {
                    node: NodeId::from(3_u64),
                    items: vec![model],
                }),
            ),
        ]),
    });
    let proxy = SpendValue::Mapping(SpendMapping {
        node: NodeId::from(4_u64),
        entries: IndexMap::from([("body".to_owned(), body)]),
    });
    let params = SpendMapping {
        node: NodeId::from(5_u64),
        entries: IndexMap::from([("proxy_server_request".to_owned(), proxy)]),
    };
    assert_eq!(
        stored(&params, "group", "gpt"),
        r#"{"proxy_server_request": {"body": {"model": "unknown-model", "messages": [{"content": "foreign"}]}}}"#
    );
}

#[test]
fn metadata_helper_alone_replaces_only_existing_keys() {
    let body = mapping(r#"{"metadata": {"model_group": "old", "other": 2}}"#);
    let placeholdered =
        placeholder_request_metadata(&body, "new", "raw", &mut NodeAllocator::default());
    assert_eq!(
        python_json::dumps(&to_json(&SpendValue::Mapping(placeholdered))),
        r#"{"metadata": {"model_group": "new", "other": 2}}"#
    );
}
