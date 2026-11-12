#![cfg(test)]

use rstest::rstest;

use super::{base, params, parse_pair};
use crate::embedding_optional_params::embedding_optional_params;
use crate::embedding_optional_params::error::EmbeddingParamsError;
use crate::non_default_params::Params;
use crate::non_default_params::mapper::DropPolicy;

#[rstest]
#[case::small("text-embedding-3-small")]
#[case::large("text-embedding-3-large")]
#[case::bare("text-embedding-3")]
#[case::suffix("text-embedding-30")]
fn openai_dimensions_pass_for_text_embedding_3_models(#[case] model: &str) {
    let (passed, special) = parse_pair(r#"{"dimensions": 8}"#, "{}");
    let request = base(model, "openai", &passed, &special);
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, params(r#"{"dimensions": 8}"#));
}

#[rstest]
#[case::ada("text-embedding-ada-002")]
#[case::other("embed-english-v3.0")]
fn openai_dimensions_are_refused_for_other_models(#[case] model: &str) {
    let (passed, special) = parse_pair(r#"{"dimensions": 8}"#, "{}");
    let request = base(model, "openai", &passed, &special);
    let error = embedding_optional_params(&request).unwrap_err();
    assert!(matches!(
        &error,
        EmbeddingParamsError::DimensionsUnsupported { .. }
    ));
    assert_eq!(error.status_code(), 400);
}

#[rstest]
#[case::call_level(DropPolicy { drop_params: Some(true), litellm_drop_params: false })]
#[case::global(DropPolicy { drop_params: Some(false), litellm_drop_params: true })]
fn openai_dimensions_are_dropped_by_either_drop_flag(#[case] drop_policy: DropPolicy) {
    let (passed, special) = parse_pair(r#"{"dimensions": 8}"#, "{}");
    let mut request = base("text-embedding-ada-002", "openai", &passed, &special);
    request.drop_policy = drop_policy;
    assert_eq!(embedding_optional_params(&request).unwrap(), Params::new());
}

#[test]
fn openai_dimensions_are_kept_when_allowed_openai_params_names_them() {
    let (passed, special) = parse_pair(r#"{"dimensions": 8}"#, "{}");
    let allowed = ["dimensions".to_owned()];
    let mut request = base("text-embedding-ada-002", "openai", &passed, &special);
    request.allowed_openai_params = &allowed;
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, params(r#"{"dimensions": 8}"#));
}

#[test]
fn openai_dimensions_are_checked_only_when_the_parameter_is_present() {
    let (passed, special) = parse_pair(r#"{"user": "u"}"#, "{}");
    let request = base("text-embedding-ada-002", "openai", &passed, &special);
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, params(r#"{"user": "u"}"#));
}

#[test]
fn openai_other_params_survive_a_dropped_dimension() {
    let (passed, special) = parse_pair(r#"{"dimensions": 8, "user": "u"}"#, "{}");
    let mut request = base("text-embedding-ada-002", "openai", &passed, &special);
    request.drop_policy = DropPolicy {
        drop_params: Some(true),
        litellm_drop_params: false,
    };
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, params(r#"{"user": "u"}"#));
}

#[test]
fn openai_dimensions_ignore_an_allowed_name_that_is_not_dimensions() {
    let (passed, special) = parse_pair(r#"{"dimensions": 8}"#, "{}");
    let allowed = ["temperature".to_owned()];
    let mut request = base("text-embedding-ada-002", "openai", &passed, &special);
    request.allowed_openai_params = &allowed;
    let error = embedding_optional_params(&request).unwrap_err();
    assert!(matches!(
        &error,
        EmbeddingParamsError::DimensionsUnsupported { .. }
    ));
}

#[test]
fn azure_sends_dimensions_without_the_text_embedding_3_check() {
    let (passed, special) = parse_pair(r#"{"dimensions": 8}"#, "{}");
    let request = base("text-embedding-ada-002", "azure", &passed, &special);
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, params(r#"{"dimensions": 8}"#));
}

#[test]
fn openai_dimensions_zero_is_used_as_given() {
    let (passed, special) = parse_pair(r#"{"dimensions": 0}"#, "{}");
    let request = base("text-embedding-3-small", "openai", &passed, &special);
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, params(r#"{"dimensions": 0}"#));
}
