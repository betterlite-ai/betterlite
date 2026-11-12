#![cfg(test)]

use std::sync::Mutex;

use betterlite_models::controls::json_value::JsonValue;
use indexmap::IndexMap;
use rstest::rstest;

use super::{FixtureMapper, MapperSeen, base, params, parse_pair};
use crate::embedding_optional_params::embedding_optional_params;
use crate::embedding_optional_params::error::EmbeddingParamsError;
use crate::non_default_params::Params;
use crate::non_default_params::mapper::DropPolicy;

#[test]
fn config_path_restores_a_supported_provider_only_param() {
    let (passed, special) = parse_pair(r#"{"user": "u", "timeout": 30}"#, "{}");
    let fixture = FixtureMapper::mapper(&["user", "timeout"], r#"{"user": "u"}"#);
    let mut request = base("m", "hosted_vllm", &passed, &special);
    request.provider_config = Some(&fixture);
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(
        result,
        params(r#"{"user": "u", "timeout": 30, "extra_body": {"timeout": 30}}"#)
    );
}

#[test]
fn config_path_does_not_restore_an_openai_embedding_param() {
    let (passed, special) = parse_pair(r#"{"dimensions": 8}"#, "{}");
    let fixture = FixtureMapper::mapper(&["dimensions"], "{}");
    let mut request = base("m", "hosted_vllm", &passed, &special);
    request.provider_config = Some(&fixture);
    assert_eq!(embedding_optional_params(&request).unwrap(), Params::new());
}

#[rstest]
#[case::null_value(r#"{"timeout": null}"#, "{}")]
#[case::already_mapped(r#"{"timeout": 2}"#, r#"{"timeout": 1, "extra_body": {"timeout": 2}}"#)]
fn config_path_does_not_restore_a_null_or_mapped_param(
    #[case] passed_json: &str,
    #[case] expected: &str,
) {
    let (passed, special) = parse_pair(passed_json, "{}");
    let fixture = FixtureMapper::mapper(&["timeout"], r#"{"timeout": 1}"#);
    let mut request = base("m", "hosted_vllm", &passed, &special);
    request.provider_config = Some(&fixture);
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, params(expected));
}

#[test]
fn config_path_restores_nothing_without_a_supported_list() {
    let (passed, special) = parse_pair(r#"{"timeout": 30}"#, "{}");
    let fixture = FixtureMapper::mapper(&[], "{}");
    let mut request = base("m", "hosted_vllm", &passed, &special);
    request.provider_config = Some(&fixture);
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, params(r#"{"extra_body": {"timeout": 30}}"#));
}

#[rstest]
#[case::refused(DropPolicy::default(), false)]
#[case::call_level(DropPolicy { drop_params: Some(true), litellm_drop_params: false }, true)]
#[case::global(DropPolicy { drop_params: None, litellm_drop_params: true }, true)]
fn config_path_drops_an_unsupported_param_only_with_drop_params(
    #[case] drop_policy: DropPolicy,
    #[case] accepted: bool,
) {
    let (passed, special) = parse_pair(r#"{"user": "u"}"#, "{}");
    let fixture = FixtureMapper::mapper(&["dimensions"], "{}");
    let mut request = base("m", "hosted_vllm", &passed, &special);
    request.provider_config = Some(&fixture);
    request.drop_policy = drop_policy;
    if accepted {
        assert_eq!(embedding_optional_params(&request).unwrap(), Params::new());
        assert_eq!(fixture.seen()[0].non_default, params(r#"{"user": "u"}"#));
    } else {
        let error = embedding_optional_params(&request).unwrap_err();
        assert!(matches!(&error, EmbeddingParamsError::Unsupported { .. }));
        assert_eq!(error.status_code(), 400);
        assert!(fixture.seen().is_empty());
    }
}

#[rstest]
#[case::call_level(DropPolicy { drop_params: Some(true), litellm_drop_params: false }, true)]
#[case::global_alone(DropPolicy { drop_params: None, litellm_drop_params: true }, false)]
fn config_path_maps_with_an_empty_optional_params_and_the_per_call_drop(
    #[case] drop_policy: DropPolicy,
    #[case] seen_drop: bool,
) {
    let (passed, special) = parse_pair(r#"{"user": "u"}"#, "{}");
    let fixture = FixtureMapper::mapper(&["user"], r#"{"mapped": true}"#);
    let mut request = base("model-x", "hosted_vllm", &passed, &special);
    request.provider_config = Some(&fixture);
    request.drop_policy = drop_policy;
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, params(r#"{"mapped": true}"#));
    assert_eq!(
        fixture.seen(),
        vec![MapperSeen {
            non_default: params(r#"{"user": "u"}"#),
            optional: Params::new(),
            model: "model-x".to_owned(),
            drop_params: seen_drop,
        }]
    );
}

#[test]
fn config_path_keeps_the_mapper_error_status() {
    let (passed, special) = parse_pair(r#"{"user": "u"}"#, "{}");
    let fixture = FixtureMapper {
        supported: vec!["user".to_owned()],
        mapped: Params::new(),
        fail_status: Some(502),
        seen: Mutex::new(Vec::new()),
    };
    let mut request = base("m", "hosted_vllm", &passed, &special);
    request.provider_config = Some(&fixture);
    let error = embedding_optional_params(&request).unwrap_err();
    assert!(matches!(&error, EmbeddingParamsError::Mapper(_)));
    assert_eq!(error.status_code(), 502);
}

#[test]
fn config_path_reports_a_deep_extra_body_as_a_params_error() {
    let (passed, mut special) = parse_pair("{}", "{}");
    special.insert("extra_body".to_owned(), nested_object(102));
    let fixture = FixtureMapper::mapper(&[], "{}");
    let mut request = base("m", "openai", &passed, &special);
    request.provider_config = Some(&fixture);
    let error = embedding_optional_params(&request).unwrap_err();
    assert!(matches!(&error, EmbeddingParamsError::Params(_)));
    assert_eq!(error.status_code(), 400);
}

fn nested_object(depth: usize) -> JsonValue {
    let mut value = JsonValue::Object(IndexMap::new());
    for _ in 0..depth {
        let mut outer = IndexMap::new();
        outer.insert("nested".to_owned(), value);
        value = JsonValue::Object(outer);
    }
    value
}
