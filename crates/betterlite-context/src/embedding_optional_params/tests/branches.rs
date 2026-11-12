#![cfg(test)]

use rstest::rstest;

use super::{FixtureMapper, MapperSeen, base, params, parse_pair};
use crate::embedding_optional_params::embedding_optional_params;
use crate::embedding_optional_params::error::EmbeddingParamsError;
use crate::non_default_params::Params;
use crate::non_default_params::mapper::DropPolicy;

#[rstest]
#[case::triton("triton")]
#[case::databricks("databricks")]
#[case::nvidia_nim("nvidia_nim")]
#[case::mistral("mistral")]
#[case::jina_ai("jina_ai")]
#[case::sap("sap")]
#[case::fireworks_ai("fireworks_ai")]
#[case::sambanova("sambanova")]
#[case::ovhcloud("ovhcloud")]
fn provider_branches_delegate_to_the_mapper(#[case] provider: &str) {
    let (passed, special) = parse_pair(r#"{"user": "u"}"#, "{}");
    let fixture = FixtureMapper::mapper(&["user"], r#"{"mapped": true}"#);
    let supported = ["user".to_owned()];
    let mut request = base("model-x", provider, &passed, &special);
    request.mapper = Some(&fixture);
    request.supported = Some(&supported);
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, params(r#"{"mapped": true}"#));
    assert_eq!(
        fixture.seen(),
        vec![MapperSeen {
            non_default: params(r#"{"user": "u"}"#),
            optional: Params::new(),
            model: "model-x".to_owned(),
            drop_params: false,
        }]
    );
}

#[test]
fn provider_branch_rejects_an_unsupported_param() {
    let (passed, special) = parse_pair(r#"{"user": "u"}"#, "{}");
    let fixture = FixtureMapper::mapper(&[], r#"{"mapped": true}"#);
    let supported: [String; 0] = [];
    let mut request = base("m", "triton", &passed, &special);
    request.mapper = Some(&fixture);
    request.supported = Some(&supported);
    let error = embedding_optional_params(&request).unwrap_err();
    assert!(matches!(&error, EmbeddingParamsError::Unsupported { .. }));
    assert_eq!(error.status_code(), 400);
    assert!(fixture.seen().is_empty());
}

#[test]
fn provider_branch_drops_an_unsupported_param_but_still_maps_it() {
    let (passed, special) = parse_pair(r#"{"user": "u"}"#, "{}");
    let fixture = FixtureMapper::mapper(&[], r#"{"mapped": true}"#);
    let mut request = base("m", "triton", &passed, &special);
    request.mapper = Some(&fixture);
    request.drop_policy = DropPolicy {
        drop_params: Some(true),
        litellm_drop_params: false,
    };
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, params(r#"{"mapped": true}"#));
    assert_eq!(fixture.seen()[0].non_default, params(r#"{"user": "u"}"#));
}

#[test]
fn provider_branch_without_a_supported_answer_skips_the_check() {
    let (passed, special) = parse_pair(r#"{"user": "u"}"#, "{}");
    let fixture = FixtureMapper::mapper(&[], "{}");
    let mut request = base("m", "ovhcloud", &passed, &special);
    request.mapper = Some(&fixture);
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, Params::new());
    assert_eq!(fixture.seen()[0].non_default, params(r#"{"user": "u"}"#));
}

#[rstest]
#[case::float_is_dropped("float", None)]
#[case::base64_is_refused("base64", Some(400))]
fn vertex_branch_checks_encoding_format_before_the_supported_list(
    #[case] encoding_format: &str,
    #[case] error_status: Option<u16>,
) {
    let passed_json = format!(r#"{{"encoding_format": "{encoding_format}", "dimensions": 8}}"#);
    let (passed, special) = parse_pair(&passed_json, "{}");
    let fixture = FixtureMapper::mapper(&["dimensions"], r#"{"outputDimensionality": 8}"#);
    let supported = ["dimensions".to_owned()];
    let mut request = base("text-embedding-004", "vertex_ai", &passed, &special);
    request.mapper = Some(&fixture);
    request.supported = Some(&supported);
    match error_status {
        None => {
            let result = embedding_optional_params(&request).unwrap();
            assert_eq!(result, params(r#"{"outputDimensionality": 8}"#));
            assert_eq!(
                fixture.seen()[0].non_default,
                params(r#"{"dimensions": 8}"#)
            );
        }
        Some(status) => {
            let error = embedding_optional_params(&request).unwrap_err();
            assert!(matches!(&error, EmbeddingParamsError::Unsupported { .. }));
            assert_eq!(error.status_code(), status);
        }
    }
}

#[test]
fn gemini_branch_uses_the_vertex_supported_dimensions() {
    let (passed, special) = parse_pair(r#"{"dimensions": 8}"#, "{}");
    let fixture = FixtureMapper::mapper(&["dimensions"], r#"{"outputDimensionality": 8}"#);
    let supported = ["dimensions".to_owned()];
    let mut request = base("text-embedding-004", "gemini", &passed, &special);
    request.mapper = Some(&fixture);
    request.supported = Some(&supported);
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, params(r#"{"outputDimensionality": 8}"#));
}

#[test]
fn lm_studio_branch_uses_the_mappers_supported_list() {
    let (passed, special) = parse_pair(r#"{"dimensions": 8}"#, "{}");
    let fixture = FixtureMapper::mapper(&["dimensions"], r#"{"mapped": true}"#);
    let mut request = base("local-model", "lm_studio", &passed, &special);
    request.mapper = Some(&fixture);
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, params(r#"{"mapped": true}"#));
}

#[test]
fn lm_studio_branch_rejects_a_param_the_mapper_does_not_support() {
    let (passed, special) = parse_pair(r#"{"user": "u"}"#, "{}");
    let fixture = FixtureMapper::mapper(&[], "{}");
    let mut request = base("local-model", "lm_studio", &passed, &special);
    request.mapper = Some(&fixture);
    let error = embedding_optional_params(&request).unwrap_err();
    assert!(matches!(&error, EmbeddingParamsError::Unsupported { .. }));
}

#[rstest]
#[case::voyage("voyage")]
#[case::infinity("infinity")]
fn voyage_and_infinity_branches_merge_the_special_params_over_the_map(#[case] provider: &str) {
    let (passed, special) = parse_pair(
        r#"{"dimensions": 8}"#,
        r#"{"custom_field": 1, "extra_body": {}, "output_dimension": 3}"#,
    );
    let fixture = FixtureMapper::mapper(&["dimensions"], r#"{"output_dimension": 8}"#);
    let supported = ["dimensions".to_owned()];
    let mut request = base(provider, provider, &passed, &special);
    request.mapper = Some(&fixture);
    request.supported = Some(&supported);
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(
        result,
        params(r#"{"custom_field": 1, "extra_body": {}, "output_dimension": 3}"#)
    );
}

#[test]
fn ollama_branch_keeps_dimensions_and_rejects_the_rest() {
    let (passed, special) = parse_pair(r#"{"dimensions": 2, "user": "u"}"#, "{}");
    let request = base("nomic-embed-text", "ollama", &passed, &special);
    let error = embedding_optional_params(&request).unwrap_err();
    assert!(matches!(&error, EmbeddingParamsError::Unsupported { .. }));
    assert_eq!(error.status_code(), 400);
}

#[test]
fn ollama_branch_drops_the_rest_with_drop_params() {
    let (passed, special) = parse_pair(r#"{"dimensions": 2, "user": "u"}"#, "{}");
    let mut request = base("nomic-embed-text", "ollama", &passed, &special);
    request.drop_policy = DropPolicy {
        drop_params: Some(true),
        litellm_drop_params: false,
    };
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, params(r#"{"dimensions": 2}"#));
}

#[test]
fn ollama_branch_returns_nothing_without_non_default_params() {
    let (passed, special) = parse_pair("{}", "{}");
    let request = base("nomic-embed-text", "ollama", &passed, &special);
    assert_eq!(embedding_optional_params(&request).unwrap(), Params::new());
}

#[test]
fn fallback_branch_rejects_a_non_default_param() {
    let (passed, special) = parse_pair(r#"{"user": "u"}"#, "{}");
    let request = base("m", "own_provider", &passed, &special);
    let error = embedding_optional_params(&request).unwrap_err();
    assert!(matches!(&error, EmbeddingParamsError::Unsupported { .. }));
}

#[rstest]
#[case::kept_without_non_default("{}", r#"{"foo": 1}"#, r#"{"foo": 1}"#)]
#[case::dropped_with_drop_params(r#"{"user": "u"}"#, r#"{"foo": 1}"#, r#"{"foo": 1}"#)]
fn fallback_branch_adds_the_passed_extras(
    #[case] passed_json: &str,
    #[case] special_json: &str,
    #[case] expected: &str,
) {
    let (passed, special) = parse_pair(passed_json, special_json);
    let mut request = base("m", "own_provider", &passed, &special);
    request.drop_policy = DropPolicy {
        drop_params: Some(true),
        litellm_drop_params: false,
    };
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, params(expected));
}

#[test]
fn fallback_branch_keeps_an_extra_body() {
    let (passed, special) = parse_pair("{}", r#"{"extra_body": {"a": 1}}"#);
    let request = base("m", "own_provider", &passed, &special);
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, params(r#"{"extra_body": {"a": 1}}"#));
}

#[test]
fn openai_compatible_branch_puts_unknown_params_in_extra_body() {
    let (passed, special) = parse_pair(r#"{"user": "u"}"#, r#"{"custom_field": 1}"#);
    let request = base("m", "groq", &passed, &special);
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(
        result,
        params(r#"{"user": "u", "extra_body": {"custom_field": 1}}"#)
    );
}

#[test]
fn azure_branch_passes_the_non_default_params_through() {
    let (passed, special) = parse_pair(r#"{"dimensions": 8, "user": "u"}"#, "{}");
    let request = base("text-embedding-ada-002", "azure", &passed, &special);
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, params(r#"{"dimensions": 8, "user": "u"}"#));
}

#[rstest]
#[case::empty("{}", "{}")]
#[case::empty_object(r#"{"extra_body": {}}"#, "{}")]
#[case::non_empty(r#"{"extra_body": {"a": 1}}"#, r#"{"extra_body": {"a": 1}}"#)]
#[case::null(r#"{"extra_body": null}"#, "{}")]
fn empty_extra_body_is_removed_from_the_output(#[case] special_json: &str, #[case] expected: &str) {
    let (passed, special) = parse_pair("{}", special_json);
    let request = base("text-embedding-3-small", "openai", &passed, &special);
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, params(expected));
}

#[test]
fn wrong_type_extra_body_is_a_field_error() {
    let (passed, special) = parse_pair("{}", r#"{"extra_body": 5}"#);
    let request = base("text-embedding-3-small", "openai", &passed, &special);
    let error = embedding_optional_params(&request).unwrap_err();
    assert!(matches!(
        &error,
        EmbeddingParamsError::WrongType("extra_body", _)
    ));
    assert_eq!(error.status_code(), 400);
}

#[test]
fn a_branch_that_needs_a_mapper_reports_its_absence() {
    let (passed, special) = parse_pair("{}", "{}");
    let request = base("m", "triton", &passed, &special);
    let error = embedding_optional_params(&request).unwrap_err();
    assert!(matches!(&error, EmbeddingParamsError::MissingMapper { .. }));
    assert_eq!(error.status_code(), 500);
}

#[rstest]
#[case::empty_array("[]", true)]
#[case::empty_text(r#""""#, true)]
#[case::non_empty_array("[1]", false)]
#[case::number("5", false)]
fn provider_extra_body_cleanup_follows_the_python_length(
    #[case] extra_body: &str,
    #[case] removed: bool,
) {
    let (passed, special) = parse_pair("{}", "{}");
    let mapped = format!(r#"{{"extra_body": {extra_body}}}"#);
    let fixture = FixtureMapper::mapper(&[], &mapped);
    let mut request = base("m", "cohere", &passed, &special);
    request.provider_config = Some(&fixture);
    let result = embedding_optional_params(&request).unwrap();
    let expected = if removed {
        Params::new()
    } else {
        params(&mapped)
    };
    assert_eq!(result, expected);
}
