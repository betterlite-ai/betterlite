use rstest::rstest;

use super::support::{float, integer, kwargs, object, text};
use crate::cache_key::value::{KeyObject, KeyValue};
use crate::handler::HandlerError;
use crate::handler::kwargs::{
    LOGGING_OBJECT_KWARG, convert_args_to_kwargs, drop_logging_obj_from_kwargs, key_request,
    no_store_requested, remove_null_metadata, request_cache_key, request_control, request_stream,
};
use crate::types::control::DynamicCacheControl;

#[test]
fn key_request_splits_named_fields_and_keeps_request_order() {
    let request = key_request(&kwargs(&[
        ("model", text("gpt-4")),
        ("metadata", object(&[("model_group", text("group"))])),
        ("messages", KeyValue::Array(Vec::new())),
        (
            "litellm_params",
            object(&[("preset_cache_key", text("preset"))]),
        ),
        (
            "cache",
            object(&[
                ("namespace", text("ns")),
                ("use-cache", KeyValue::Bool(true)),
            ]),
        ),
        ("input", text("alpha")),
    ]))
    .expect("the key request builds");
    let names: Vec<&str> = request
        .params
        .iter()
        .map(|param| param.name.as_str())
        .collect();
    assert_eq!(names, ["model", "messages", "input"]);
    let metadata = request.metadata.expect("the metadata is split out");
    assert_eq!(metadata.get("model_group"), Some(&text("group")));
    let litellm_params = request.litellm_params.expect("litellm_params is split out");
    assert_eq!(litellm_params.preset_cache_key.as_deref(), Some("preset"));
    let control = request.cache_control.expect("the cache dict is split out");
    assert_eq!(control.namespace.as_deref(), Some("ns"));
    assert!(control.use_cache);
}

#[test]
fn key_request_treats_null_named_fields_as_absent() {
    let request = key_request(&kwargs(&[
        ("model", text("gpt-4")),
        ("metadata", KeyValue::Null),
        ("litellm_metadata", KeyValue::Null),
        ("litellm_params", KeyValue::Null),
        ("cache", KeyValue::Null),
    ]))
    .expect("the key request builds");
    assert!(request.metadata.is_none());
    assert!(request.litellm_metadata.is_none());
    assert!(request.litellm_params.is_none());
    assert!(request.cache_control.is_none());
    assert_eq!(request.params.len(), 1);
}

#[rstest]
#[case::metadata_text("metadata", text("bad"))]
#[case::litellm_metadata_text("litellm_metadata", text("bad"))]
#[case::litellm_params_text("litellm_params", text("bad"))]
#[case::cache_text("cache", text("bad"))]
fn key_request_rejects_wrong_named_field_types(#[case] field: &str, #[case] value: KeyValue) {
    let request = key_request(&kwargs(&[("model", text("gpt-4")), (field, value)]));
    assert!(matches!(
        request,
        Err(HandlerError::FieldType { field: rejected, .. }) if rejected == field
    ));
}

#[rstest]
#[case::preset_key_integer(
    object(&[("preset_cache_key", integer(1))]),
    "the litellm_params.preset_cache_key value must be a string"
)]
#[case::params_metadata_text(
    object(&[("metadata", text("bad"))]),
    "the litellm_params.metadata value must be an object"
)]
fn key_request_rejects_wrong_litellm_param_types(
    #[case] litellm_params: KeyValue,
    #[case] expected: &str,
) {
    let error = key_request(&kwargs(&[
        ("model", text("gpt-4")),
        ("litellm_params", litellm_params),
    ]))
    .expect_err("the wrong type is rejected");
    assert_eq!(error.to_string(), expected);
}

#[test]
fn convert_args_to_kwargs_uses_parameter_order() {
    let parameters = ["model".to_owned(), "messages".to_owned()];
    let projected =
        convert_args_to_kwargs(&parameters, &[text("gpt-4"), KeyValue::Array(Vec::new())]);
    let names: Vec<&str> = projected.keys().map(String::as_str).collect();
    assert_eq!(names, ["model", "messages"]);
    assert_eq!(projected.get("model"), Some(&text("gpt-4")));
}

#[test]
fn convert_args_to_kwargs_drops_arguments_past_the_signature() {
    let parameters = ["model".to_owned()];
    let projected = convert_args_to_kwargs(&parameters, &[text("gpt-4"), text("extra")]);
    assert_eq!(projected.len(), 1);
    assert_eq!(projected.get("model"), Some(&text("gpt-4")));
}

#[test]
fn convert_args_to_kwargs_without_arguments_is_empty() {
    assert!(convert_args_to_kwargs(&[], &[]).is_empty());
}

#[test]
fn drop_logging_obj_keeps_the_other_keys() {
    let dropped = drop_logging_obj_from_kwargs(kwargs(&[
        ("model", text("gpt-4")),
        (LOGGING_OBJECT_KWARG, text("a-logging-object")),
    ]));
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped.get("model"), Some(&text("gpt-4")));
}

#[test]
fn request_control_parses_every_field() {
    let control = request_control(&kwargs(&[(
        "cache",
        object(&[
            ("ttl", integer(60)),
            ("namespace", text("ns")),
            ("s-maxage", integer(5)),
            ("s-max-age", integer(7)),
            ("no-cache", KeyValue::Bool(true)),
            ("no-store", KeyValue::Bool(true)),
        ]),
    )]))
    .expect("the control parses");
    assert_eq!(
        control,
        DynamicCacheControl {
            ttl: Some(60),
            namespace: Some("ns".to_owned()),
            s_maxage: Some(5),
            s_max_age: Some(7),
            no_cache: Some(true),
            no_store: Some(true),
        }
    );
}

#[rstest]
#[case::absent(kwargs(&[("model", text("gpt-4"))]))]
#[case::null(kwargs(&[("model", text("gpt-4")), ("cache", KeyValue::Null)]))]
fn request_control_takes_the_default_for_an_absent_cache(#[case] request_kwargs: KeyObject) {
    assert_eq!(
        request_control(&request_kwargs).expect("the control parses"),
        DynamicCacheControl::default()
    );
}

#[test]
fn request_control_ignores_unknown_cache_keys() {
    let control = request_control(&kwargs(&[(
        "cache",
        object(&[("a-future-control", integer(1))]),
    )]))
    .expect("the control parses");
    assert_eq!(control, DynamicCacheControl::default());
}

#[rstest]
#[case::ttl_float(object(&[("ttl", float(1.5))]), "cache.ttl")]
#[case::namespace_integer(object(&[("namespace", integer(1))]), "cache.namespace")]
#[case::s_maxage_text(object(&[("s-maxage", text("5"))]), "cache.s-maxage")]
#[case::s_max_age_text(object(&[("s-max-age", text("7"))]), "cache.s-max-age")]
#[case::no_cache_text(object(&[("no-cache", text("yes"))]), "cache.no-cache")]
#[case::no_store_text(object(&[("no-store", text("yes"))]), "cache.no-store")]
fn request_control_rejects_wrong_field_types(
    #[case] cache: KeyValue,
    #[case] expected_field: &str,
) {
    let result = request_control(&kwargs(&[("model", text("gpt-4")), ("cache", cache)]));
    assert!(matches!(
        result,
        Err(HandlerError::FieldType { field, .. }) if field == expected_field
    ));
}

#[rstest]
#[case::absent(kwargs(&[("model", text("gpt-4"))]), false)]
#[case::null(kwargs(&[("model", text("gpt-4")), ("stream", KeyValue::Null)]), false)]
#[case::flag_false(kwargs(&[("model", text("gpt-4")), ("stream", KeyValue::Bool(false))]), false)]
#[case::flag_true(kwargs(&[("model", text("gpt-4")), ("stream", KeyValue::Bool(true))]), true)]
fn request_stream_reads_the_boolean(#[case] request_kwargs: KeyObject, #[case] expected: bool) {
    assert_eq!(
        request_stream(&request_kwargs).expect("the stream flag reads"),
        expected
    );
}

#[test]
fn request_stream_rejects_a_non_boolean() {
    let result = request_stream(&kwargs(&[("stream", text("yes"))]));
    assert!(matches!(
        result,
        Err(HandlerError::FieldType {
            field: "stream",
            ..
        })
    ));
}

#[rstest]
#[case::absent(kwargs(&[("model", text("gpt-4"))]), None)]
#[case::null(kwargs(&[("model", text("gpt-4")), ("cache_key", KeyValue::Null)]), None)]
#[case::text(
    kwargs(&[("model", text("gpt-4")), ("cache_key", text("caller-key"))]),
    Some("caller-key")
)]
fn request_cache_key_reads_the_string(
    #[case] request_kwargs: KeyObject,
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        request_cache_key(&request_kwargs).expect("the cache key reads"),
        expected.map(str::to_owned)
    );
}

#[test]
fn request_cache_key_rejects_a_non_string() {
    let result = request_cache_key(&kwargs(&[("cache_key", integer(7))]));
    assert!(matches!(
        result,
        Err(HandlerError::FieldType {
            field: "cache_key",
            ..
        })
    ));
}

#[test]
fn remove_null_metadata_drops_absent_and_null_values_only() {
    let mut absent = kwargs(&[("model", text("gpt-4"))]);
    remove_null_metadata(&mut absent);
    assert!(!absent.contains_key("metadata"));
    let mut null = kwargs(&[("model", text("gpt-4")), ("metadata", KeyValue::Null)]);
    remove_null_metadata(&mut null);
    assert!(!null.contains_key("metadata"));
    let mut present = kwargs(&[
        ("model", text("gpt-4")),
        ("metadata", object(&[("team", text("a"))])),
    ]);
    remove_null_metadata(&mut present);
    assert!(present.contains_key("metadata"));
}

#[rstest]
#[case::absent(kwargs(&[("model", text("gpt-4"))]), false)]
#[case::null_cache(kwargs(&[("model", text("gpt-4")), ("cache", KeyValue::Null)]), false)]
#[case::empty_cache(kwargs(&[("model", text("gpt-4")), ("cache", object(&[]))]), false)]
#[case::no_store_false(
    kwargs(&[("model", text("gpt-4")), ("cache", object(&[("no-store", KeyValue::Bool(false))]))]),
    false
)]
#[case::no_store_true(
    kwargs(&[("model", text("gpt-4")), ("cache", object(&[("no-store", KeyValue::Bool(true))]))]),
    true
)]
#[case::no_store_null(
    kwargs(&[("model", text("gpt-4")), ("cache", object(&[("no-store", KeyValue::Null)]))]),
    false
)]
fn no_store_reads_the_control(#[case] request_kwargs: KeyObject, #[case] expected: bool) {
    assert_eq!(
        no_store_requested(&request_kwargs).expect("the control reads"),
        expected
    );
}
