#![cfg(test)]

use serde_json::{Value, json};

use super::{NvidiaNimEmbeddingError, SUPPORTED_OPENAI_PARAMS, map_params};
use crate::openai_base_config::Object;

fn object(value: Value) -> Object {
    match value {
        Value::Object(fields) => fields,
        _ => panic!("the fixture is not an object"),
    }
}

fn mapped(non_default: Value, optional: Value, kwargs: Option<Value>) -> Object {
    let kwargs = kwargs.map(object);
    map_params(&object(non_default), object(optional), kwargs.as_ref()).unwrap()
}

#[test]
fn supported_params_are_the_three_litellm_names() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        ["encoding_format", "user", "dimensions"]
    );
}

#[test]
fn map_params_adds_an_empty_extra_body_when_the_options_have_none() {
    let mapped = mapped(json!({}), json!({}), None);
    assert_eq!(mapped, object(json!({"extra_body": {}})));
}

#[test]
fn map_params_folds_input_type_and_truncate_into_extra_body() {
    let mapped = mapped(
        json!({"input_type": "query", "truncate": "END"}),
        json!({}),
        None,
    );
    assert_eq!(
        mapped,
        object(json!({"extra_body": {"input_type": "query", "truncate": "END"}}))
    );
}

#[test]
fn map_params_writes_every_other_key_at_the_top_level() {
    let mapped = mapped(
        json!({"dimensions": 3, "encoding_format": "float", "user": "someone"}),
        json!({}),
        None,
    );
    assert_eq!(
        mapped,
        object(json!({
            "extra_body": {},
            "dimensions": 3,
            "encoding_format": "float",
            "user": "someone"
        }))
    );
}

#[test]
fn map_params_merges_the_kwargs_into_extra_body() {
    let mapped = mapped(
        json!({"input_type": "passage"}),
        json!({}),
        Some(json!({"truncate": "NONE", "other": 1})),
    );
    assert_eq!(
        mapped,
        object(json!({
            "extra_body": {"input_type": "passage", "truncate": "NONE", "other": 1}
        }))
    );
}

#[test]
fn map_params_keeps_extra_body_present_for_an_empty_kwargs_map() {
    let mapped = mapped(json!({}), json!({}), Some(json!({})));
    assert_eq!(mapped, object(json!({"extra_body": {}})));
}

#[test]
fn map_params_merges_into_an_extra_body_the_options_already_had() {
    let mapped = mapped(
        json!({"input_type": "query"}),
        json!({"extra_body": {"mode": "kept"}}),
        Some(json!({"truncate": "END"})),
    );
    assert_eq!(
        mapped,
        object(json!({
            "extra_body": {"mode": "kept", "input_type": "query", "truncate": "END"}
        }))
    );
}

#[test]
fn map_params_merges_kwargs_into_an_extra_body_the_request_replaces() {
    let mapped = mapped(
        json!({"extra_body": {"only": "replacement"}, "user": "someone"}),
        json!({}),
        Some(json!({"truncate": "END"})),
    );
    assert_eq!(
        mapped,
        object(json!({
            "extra_body": {"only": "replacement", "truncate": "END"},
            "user": "someone"
        }))
    );
}

#[test]
fn map_params_rejects_a_non_object_extra_body_in_the_request() {
    let error = map_params(
        &object(json!({"extra_body": 5})),
        Object::new(),
        Some(&Object::new()),
    )
    .unwrap_err();
    assert_eq!(error, NvidiaNimEmbeddingError::ExtraBodyNotObject);
}

#[test]
fn map_params_rejects_a_non_object_extra_body_in_the_options() {
    let error = map_params(
        &object(json!({"input_type": "query"})),
        object(json!({"extra_body": null})),
        None,
    )
    .unwrap_err();
    assert_eq!(error, NvidiaNimEmbeddingError::ExtraBodyNotObject);
}

#[test]
fn map_params_leaves_the_requested_map_alone() {
    let non_default = object(json!({"input_type": "query", "dimensions": 2}));
    let mapped = map_params(&non_default, Object::new(), None).unwrap();
    assert_eq!(
        mapped,
        object(json!({"extra_body": {"input_type": "query"}, "dimensions": 2}))
    );
    assert_eq!(
        non_default,
        object(json!({"input_type": "query", "dimensions": 2}))
    );
}
