#![cfg(test)]

use betterlite_models::controls::json_value::JsonValue;
use rstest::rstest;

use super::params;
use crate::non_default_params::filters::{
    Endpoint, apply_openai_param_overrides, bedrock_route_for_request, filter_out_litellm_params,
    get_non_default_params, get_standard_openai_params, non_default_kwargs,
    provider_rejectable_params, remove_unsupported_params, select_non_default_params,
};
use crate::non_default_params::schema_cleanup::{
    point_refs_at_root_defs, remove_additional_properties, remove_strict_from_schema,
};

#[rstest]
#[case::null_is_default(r#"{"n": null, "user": "u"}"#, &[], r#"{"user": "u"}"#)]
#[case::zero_is_set(r#"{"n": 0}"#, &[], r#"{"n": 0}"#)]
#[case::false_is_set(r#"{"stream": false}"#, &[], r#"{"stream": false}"#)]
#[case::empty_string_is_set(r#"{"user": ""}"#, &[], r#"{"user": ""}"#)]
#[case::empty_list_is_set(r#"{"stop": []}"#, &[], r#"{"stop": []}"#)]
#[case::unknown_name_is_not_a_default(r#"{"top_k": 3, "n": 1}"#, &[], r#"{"n": 1}"#)]
#[case::dropped_by_the_caller(r#"{"n": 1, "user": "u"}"#, &["user"], r#"{"n": 1}"#)]
#[case::drop_list_does_not_touch_others(r#"{"n": 1}"#, &["user"], r#"{"n": 1}"#)]
fn select_filter_keeps_only_set_default_names(
    #[case] passed: &str,
    #[case] dropped: &[&str],
    #[case] expected: &str,
) {
    let drops: Vec<String> = dropped.iter().map(ToString::to_string).collect();
    let selected =
        select_non_default_params(&params(passed), &["n", "stop", "stream", "user"], &drops);
    assert_eq!(selected, params(expected));
}

#[test]
fn get_non_default_params_filter_excludes_routing_keys() {
    let passed = params(
        r#"{"model": "m", "custom_llm_provider": "p", "n": 2, "temperature": null, "top_k": 1, "tools": []}"#,
    );
    assert_eq!(
        get_non_default_params(&passed),
        params(r#"{"n": 2, "tools": []}"#)
    );
}

#[rstest]
#[case::user_is_unvalidated("user")]
#[case::stream_is_unvalidated("stream")]
#[case::stream_options_is_unvalidated("stream_options")]
#[case::max_retries_is_unvalidated("max_retries")]
fn provider_rejectable_filter_skips_unvalidated(#[case] name: &str) {
    let mut passed = params(r#"{"temperature": 0.5}"#);
    passed.insert(name.to_owned(), JsonValue::Bool(true));
    let rejectable = provider_rejectable_params(&passed);
    assert_eq!(rejectable.into_iter().collect::<Vec<_>>(), ["temperature"]);
}

#[rstest]
#[case::keeps_set_names(
    r#"{"temperature": 1, "top_k": 2, "n": null}"#,
    r#"{"temperature": 1}"#
)]
#[case::keeps_falsy_set_values(
    r#"{"user": "", "stream": false, "stop": []}"#,
    r#"{"user": "", "stream": false, "stop": []}"#
)]
#[case::drops_unknown_names(r#"{"api_key": "k", "top_k": 2}"#, r#"{"api_key": "k"}"#)]
fn standard_openai_filter_keeps_the_known_names(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(get_standard_openai_params(&params(input)), params(expected));
}

#[test]
fn provider_rejectable_filter_ignores_null_and_unknown() {
    let rejectable =
        provider_rejectable_params(&params(r#"{"n": null, "top_k": 1, "stop": ["x"]}"#));
    assert_eq!(rejectable.into_iter().collect::<Vec<_>>(), ["stop"]);
}

#[test]
fn bedrock_route_filter_hides_the_dropped_params() {
    let passed = params(r#"{"a": 1, "b": 2}"#);
    let seen = bedrock_route_for_request("m", &passed, &["a".to_owned()], |model, params| {
        Ok::<_, ()>((model.to_owned(), params.clone()))
    });
    assert_eq!(seen, Ok(("m".to_owned(), params(r#"{"b": 2}"#))));
}

#[test]
fn bedrock_route_filter_passes_the_route_error_on() {
    let seen = bedrock_route_for_request("m", &params("{}"), &[], |_, _| Err::<(), _>("bad route"));
    assert_eq!(seen, Err("bad route"));
}

#[rstest]
#[case::plain_names_stay(r#"{"n": 1, "top_k": 2}"#, &[], r#"{"n": 1, "top_k": 2}"#)]
#[case::excluded_name_goes(r#"{"n": 1, "top_k": 2}"#, &["n"], r#"{"top_k": 2}"#)]
#[case::litellm_owned_goes(r#"{"api_key": "k", "top_k": 2}"#, &[], r#"{"top_k": 2}"#)]
#[case::internal_prefix_goes(r#"{"_litellm_x": 1, "top_k": 2}"#, &[], r#"{"top_k": 2}"#)]
#[case::null_value_stays(r#"{"top_k": null}"#, &[], r#"{"top_k": null}"#)]
fn filter_out_litellm_params_filter_cases(
    #[case] kwargs: &str,
    #[case] excluding: &[&str],
    #[case] expected: &str,
) {
    assert_eq!(
        filter_out_litellm_params(&params(kwargs), excluding),
        params(expected)
    );
}

#[rstest]
#[case::completion(Endpoint::Completion, r#"{"extra": 3, "language": "de"}"#)]
#[case::transcription(Endpoint::Transcription, r#"{"extra": 3, "temperature": 1}"#)]
fn non_default_kwargs_filter_uses_the_endpoint_name_list(
    #[case] endpoint: Endpoint,
    #[case] expected: &str,
) {
    let kwargs = params(
        r#"{"temperature": 1, "language": "de", "extra": 3, "api_key": "k", "_litellm_x": 1}"#,
    );
    assert_eq!(non_default_kwargs(&kwargs, endpoint), params(expected));
}

#[rstest]
#[case::no_supported_list(None, "{}")]
#[case::empty_supported_list(Some(vec![]), "{}")]
#[case::keeps_the_supported(Some(vec!["n".to_owned()]), r#"{"n": 1}"#)]
fn remove_unsupported_filter_cases(#[case] supported: Option<Vec<String>>, #[case] expected: &str) {
    let result =
        remove_unsupported_params(params(r#"{"n": 1, "user": "u"}"#), supported.as_deref());
    assert_eq!(result, params(expected));
}

#[rstest]
#[case::moves_a_sent_param("{}", r#"{"user": "u"}"#, &["user"], r#"{"user": "u"}"#, "{}")]
#[case::keeps_the_mapped_value(r#"{"user": "mapped"}"#, r#"{"user": "u"}"#, &["user"], r#"{"user": "mapped"}"#, r#"{"user": "u"}"#)]
#[case::skips_an_unsent_param("{}", r#"{"n": 1}"#, &["user"], "{}", r#"{"n": 1}"#)]
#[case::leaves_others("{}", r#"{"n": 1, "user": "u"}"#, &["user"], r#"{"user": "u"}"#, r#"{"n": 1}"#)]
fn apply_overrides_filter_cases(
    #[case] optional: &str,
    #[case] non_default: &str,
    #[case] allowed_names: &[&str],
    #[case] expected_optional: &str,
    #[case] expected_rest: &str,
) {
    let allowed: Vec<String> = allowed_names.iter().map(ToString::to_string).collect();
    let mut rest = params(non_default);
    let result = apply_openai_param_overrides(params(optional), &mut rest, &allowed);
    assert_eq!(result, params(expected_optional));
    assert_eq!(rest, params(expected_rest));
}

fn schema(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

#[rstest]
#[case::false_goes(
    r#"{"additionalProperties": false, "type": "object"}"#,
    r#"{"type": "object"}"#
)]
#[case::true_stays(
    r#"{"additionalProperties": true}"#,
    r#"{"additionalProperties": true}"#
)]
#[case::schema_value_stays(
    r#"{"additionalProperties": {"type": "string"}}"#,
    r#"{"additionalProperties": {"type": "string"}}"#
)]
#[case::string_false_stays(
    r#"{"additionalProperties": "false"}"#,
    r#"{"additionalProperties": "false"}"#
)]
#[case::nested_object_and_array(
    r#"{"properties": {"a": {"additionalProperties": false}}, "anyOf": [{"additionalProperties": false, "x": 1}]}"#,
    r#"{"properties": {"a": {}}, "anyOf": [{"x": 1}]}"#
)]
fn schema_filter_removes_additional_properties_false_only(
    #[case] input: &str,
    #[case] expected: &str,
) {
    let mut value = schema(input);
    remove_additional_properties(&mut value);
    assert_eq!(value, schema(expected));
}

#[rstest]
#[case::top_level(r#"{"strict": true, "type": "object"}"#, r#"{"type": "object"}"#)]
#[case::any_value(r#"{"strict": "yes", "type": "object"}"#, r#"{"type": "object"}"#)]
#[case::nested(
    r#"{"function": {"strict": false, "parameters": [{"strict": true, "a": 1}]}}"#,
    r#"{"function": {"parameters": [{"a": 1}]}}"#
)]
fn schema_filter_removes_strict_everywhere(#[case] input: &str, #[case] expected: &str) {
    let mut value = schema(input);
    remove_strict_from_schema(&mut value);
    assert_eq!(value, schema(expected));
}

#[rstest]
#[case::nested_and_listed(
    r##"{"type": "json_schema", "json_schema": {"name": "n", "schema": {"$ref": "#/$defs/A", "properties": {"b": {"$ref": "#/definitions/B/C"}}, "anyOf": [{"$ref": "x"}]}}}"##,
    r#"{"type": "json_schema", "json_schema": {"name": "n", "schema": {"$ref": "/$defs/A", "properties": {"b": {"$ref": "/$defs/C"}}, "anyOf": [{"$ref": "/$defs/x"}]}}}"#
)]
#[case::other_format_types_stay(
    r##"{"type": "json_object", "json_schema": {"schema": {"$ref": "#/$defs/A"}}}"##,
    r##"{"type": "json_object", "json_schema": {"schema": {"$ref": "#/$defs/A"}}}"##
)]
#[case::refs_outside_the_schema_stay(
    r##"{"type": "json_schema", "json_schema": {"$ref": "#/$defs/A", "schema": {}}, "$ref": "#/$defs/B"}"##,
    r##"{"type": "json_schema", "json_schema": {"$ref": "#/$defs/A", "schema": {}}, "$ref": "#/$defs/B"}"##
)]
#[case::missing_json_schema(r#"{"type": "json_schema"}"#, r#"{"type": "json_schema"}"#)]
#[case::missing_schema(
    r#"{"type": "json_schema", "json_schema": {"name": "n"}}"#,
    r#"{"type": "json_schema", "json_schema": {"name": "n"}}"#
)]
#[case::non_text_ref_stays(
    r#"{"type": "json_schema", "json_schema": {"schema": {"$ref": 1}}}"#,
    r#"{"type": "json_schema", "json_schema": {"schema": {"$ref": 1}}}"#
)]
#[case::not_an_object(r#""json_schema""#, r#""json_schema""#)]
fn schema_filter_points_refs_at_the_root_defs(#[case] input: &str, #[case] expected: &str) {
    let mut value = schema(input);
    point_refs_at_root_defs(&mut value);
    assert_eq!(value, schema(expected));
}
