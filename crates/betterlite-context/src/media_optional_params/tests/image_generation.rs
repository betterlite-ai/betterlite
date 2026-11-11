#![cfg(test)]

use rstest::rstest;

use super::{FixtureMapper, params};
use crate::media_optional_params::error::MediaParamsError;
use crate::media_optional_params::image_generation::{
    ImageGenerationRequest, get_optional_params_image_gen, vertex_aspect_ratio,
};
use crate::non_default_params::Params;
use crate::non_default_params::mapper::DropPolicy;

const CONFIG: FixtureMapper = FixtureMapper {
    supported: &["n", "size", "foo"],
    fail_status: None,
};

const BEDROCK: FixtureMapper = FixtureMapper {
    supported: &["n", "quality"],
    fail_status: None,
};

const FAILING: FixtureMapper = FixtureMapper {
    supported: &["n"],
    fail_status: Some(502),
};

const DROP_ALL: DropPolicy = DropPolicy {
    drop_params: Some(true),
    litellm_drop_params: false,
};

fn request<'request>(
    passed: &'request Params,
    provider: Option<&'request str>,
) -> ImageGenerationRequest<'request> {
    ImageGenerationRequest {
        model: Some("m"),
        custom_llm_provider: provider,
        params: passed,
        additional_drop_params: &[],
        drop_policy: DropPolicy::default(),
        provider_config: None,
        bedrock_config: Some(&BEDROCK),
    }
}

fn generate(passed: &str, provider: Option<&str>) -> Result<Params, MediaParamsError> {
    get_optional_params_image_gen(&request(&params(passed), provider))
}

#[rstest]
#[case::wide(Some("1792x1024"), "16:9")]
#[case::tall(Some("1024x1792"), "9:16")]
#[case::square(Some("1024x1024"), "1:1")]
#[case::unknown(Some("512x512"), "1:1")]
#[case::empty(Some(""), "1:1")]
#[case::absent(None, "1:1")]
fn media_optional_params_vertex_aspect_ratio_cases(
    #[case] size: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(vertex_aspect_ratio(size), expected);
}

#[rstest]
#[case::openai("openai")]
#[case::azure("azure")]
#[case::compatible("groq")]
fn media_optional_params_image_passes_the_non_default_params_through(#[case] provider: &str) {
    let passed = r#"{"n": 1, "quality": "hd", "style": null, "tools": [], "top_k": 3}"#;
    let result = generate(passed, Some(provider)).unwrap();
    assert_eq!(
        result,
        params(r#"{"n": 1, "quality": "hd", "extra_body": {"top_k": 3}}"#)
    );
}

#[test]
fn media_optional_params_image_empty_values_are_cleaned_up() {
    let passed = r#"{"n": 0, "imageConfig": {}, "web_search_options": {"a": 1}, "tools": [1]}"#;
    let result = generate(passed, Some("openai")).unwrap();
    assert_eq!(
        result,
        params(r#"{"n": 0, "web_search_options": {"a": 1}, "tools": [1]}"#)
    );
}

#[test]
fn media_optional_params_image_openai_without_extras_has_no_extra_body() {
    assert_eq!(
        generate(r#"{"size": "256x256"}"#, Some("openai")).unwrap(),
        params(r#"{"size": "256x256"}"#)
    );
}

#[rstest]
#[case::anthropic(Some("anthropic"))]
#[case::no_provider(None)]
fn media_optional_params_image_other_providers_only_get_extra_params(
    #[case] provider: Option<&str>,
) {
    let passed = r#"{"n": 1, "size": "1x1", "top_k": 3, "unset": null}"#;
    let result = generate(passed, provider).unwrap();
    assert_eq!(result, params(r#"{"top_k": 3}"#));
}

#[test]
fn media_optional_params_image_additional_drop_params_hide_params_everywhere() {
    let passed = params(r#"{"n": 1, "user": "u", "top_k": 3, "keep": 1}"#);
    let drops = vec!["user".to_owned(), "top_k".to_owned()];
    let openai = ImageGenerationRequest {
        additional_drop_params: &drops,
        ..request(&passed, Some("openai"))
    };
    assert_eq!(
        get_optional_params_image_gen(&openai).unwrap(),
        params(r#"{"n": 1, "extra_body": {"keep": 1}}"#)
    );
    let other = ImageGenerationRequest {
        additional_drop_params: &drops,
        ..request(&passed, Some("anthropic"))
    };
    assert_eq!(
        get_optional_params_image_gen(&other).unwrap(),
        params(r#"{"keep": 1}"#)
    );
}

#[rstest]
#[case::aws_for_openai("openai", "aws_region_name", false)]
#[case::aws_for_bedrock("bedrock", "aws_region_name", true)]
#[case::aws_for_sagemaker("sagemaker", "aws_region_name", true)]
#[case::aws_without_provider_is_dropped("", "aws_region_name", false)]
#[case::hf_for_sagemaker("sagemaker", "hf_model_name", true)]
#[case::hf_for_bedrock("bedrock", "hf_model_name", false)]
#[case::vertex_for_vertex_ai("vertex_ai", "vertex_project", true)]
#[case::vertex_for_vertex_ai_beta("vertex_ai_beta", "vertex_location", true)]
#[case::vertex_for_openai("openai", "vertex_project", false)]
#[case::vertex_for_bedrock("bedrock", "vertex_project", false)]
#[case::plain_key_for_anthropic("anthropic", "some_key", true)]
fn media_optional_params_image_init_kwargs_follow_the_provider(
    #[case] provider: &str,
    #[case] key: &str,
    #[case] kept: bool,
) {
    let passed = params(&format!(r#"{{"{key}": "v"}}"#));
    let route = (!provider.is_empty()).then_some(provider);
    let result = get_optional_params_image_gen(&request(&passed, route)).unwrap();
    assert_eq!(result.contains_key(key), kept);
}

#[test]
fn media_optional_params_image_provider_config_maps_the_supported_params() {
    let passed = params(r#"{"n": 2, "foo": "f", "quality": null, "bar": 1}"#);
    let config = ImageGenerationRequest {
        provider_config: Some(&CONFIG),
        ..request(&passed, Some("other"))
    };
    let result = get_optional_params_image_gen(&config).unwrap();
    assert_eq!(
        result,
        params(r#"{"n": 2, "foo": "f", "bar": 1, "seen_drop": false, "seen_model": "m"}"#)
    );
}

#[test]
fn media_optional_params_image_provider_config_rejects_an_unsupported_param() {
    let passed = params(r#"{"n": 2, "quality": "hd"}"#);
    let config = ImageGenerationRequest {
        provider_config: Some(&CONFIG),
        ..request(&passed, Some("other"))
    };
    let error = get_optional_params_image_gen(&config).unwrap_err();
    assert!(
        matches!(&error, MediaParamsError::ImageUnsupported { param, .. } if param == "quality")
    );
    assert_eq!(error.status_code(), 400);
    assert_eq!(error.to_string(), "quality is not supported by other for m");
}

#[test]
fn media_optional_params_image_error_names_a_missing_provider_and_model_as_none() {
    let passed = params(r#"{"quality": "hd"}"#);
    let config = ImageGenerationRequest {
        model: None,
        provider_config: Some(&CONFIG),
        ..request(&passed, None)
    };
    let error = get_optional_params_image_gen(&config).unwrap_err();
    assert_eq!(
        error.to_string(),
        "quality is not supported by None for None"
    );
}

#[rstest]
#[case::call_level(DropPolicy { drop_params: Some(true), litellm_drop_params: false })]
#[case::global(DropPolicy { drop_params: None, litellm_drop_params: true })]
#[case::global_beats_call_false(DropPolicy { drop_params: Some(false), litellm_drop_params: true })]
fn media_optional_params_image_drop_removes_unsupported_params_for_good(
    #[case] drop_policy: DropPolicy,
) {
    let passed = params(r#"{"n": 2, "quality": "hd"}"#);
    let config = ImageGenerationRequest {
        provider_config: Some(&CONFIG),
        drop_policy,
        ..request(&passed, Some("other"))
    };
    let result = get_optional_params_image_gen(&config).unwrap();
    assert_eq!(
        result,
        params(r#"{"n": 2, "seen_drop": true, "seen_model": "m"}"#)
    );
}

#[test]
fn media_optional_params_image_drop_false_keeps_the_error() {
    let passed = params(r#"{"quality": "hd"}"#);
    let config = ImageGenerationRequest {
        provider_config: Some(&CONFIG),
        drop_policy: DropPolicy {
            drop_params: Some(false),
            litellm_drop_params: false,
        },
        ..request(&passed, Some("other"))
    };
    let error = get_optional_params_image_gen(&config).unwrap_err();
    assert!(matches!(error, MediaParamsError::ImageUnsupported { .. }));
}

#[test]
fn media_optional_params_image_provider_config_error_keeps_its_status() {
    let passed = params(r#"{"n": 2}"#);
    let config = ImageGenerationRequest {
        provider_config: Some(&FAILING),
        ..request(&passed, Some("other"))
    };
    let error = get_optional_params_image_gen(&config).unwrap_err();
    assert!(matches!(error, MediaParamsError::Mapper(_)));
    assert_eq!(error.status_code(), 502);
}

#[test]
fn media_optional_params_image_provider_config_beats_the_provider_branches() {
    let passed = params(r#"{"n": 2}"#);
    let config = ImageGenerationRequest {
        provider_config: Some(&CONFIG),
        ..request(&passed, Some("openai"))
    };
    let result = get_optional_params_image_gen(&config).unwrap();
    assert_eq!(
        result,
        params(r#"{"n": 2, "seen_drop": false, "seen_model": "m"}"#)
    );
}

#[test]
fn media_optional_params_image_bedrock_uses_its_config_class() {
    let passed = params(r#"{"n": 2, "quality": "hd"}"#);
    let result = get_optional_params_image_gen(&request(&passed, Some("bedrock"))).unwrap();
    assert_eq!(
        result,
        params(r#"{"n": 2, "quality": "hd", "seen_drop": false, "seen_model": "m"}"#)
    );
}

#[test]
fn media_optional_params_image_bedrock_rejects_params_its_config_lacks() {
    let passed = params(r#"{"size": "1x1"}"#);
    let error = get_optional_params_image_gen(&request(&passed, Some("bedrock"))).unwrap_err();
    assert!(
        matches!(&error, MediaParamsError::ImageUnsupported { param, provider, .. }
        if param == "size" && provider == "bedrock")
    );
}

#[test]
fn media_optional_params_image_bedrock_never_asks_the_config_to_drop() {
    let passed = params(r#"{"n": 2, "size": "1x1"}"#);
    let bedrock = ImageGenerationRequest {
        drop_policy: DROP_ALL,
        ..request(&passed, Some("bedrock"))
    };
    let result = get_optional_params_image_gen(&bedrock).unwrap();
    assert_eq!(
        result,
        params(r#"{"n": 2, "seen_drop": false, "seen_model": "m"}"#)
    );
}

#[test]
fn media_optional_params_image_bedrock_without_a_config_class_is_an_error() {
    let passed = params(r#"{"n": 2}"#);
    let bedrock = ImageGenerationRequest {
        bedrock_config: None,
        ..request(&passed, Some("bedrock"))
    };
    let error = get_optional_params_image_gen(&bedrock).unwrap_err();
    assert!(matches!(error, MediaParamsError::MissingBedrockConfig));
    assert_eq!(error.status_code(), 400);
}

#[rstest]
#[case::both(
    r#"{"n": 2, "size": "1792x1024"}"#,
    r#"{"sampleCount": 2, "aspectRatio": "16:9"}"#
)]
#[case::tall(r#"{"size": "1024x1792"}"#, r#"{"aspectRatio": "9:16"}"#)]
#[case::unknown_size(r#"{"size": "7x7"}"#, r#"{"aspectRatio": "1:1"}"#)]
#[case::count_only(r#"{"n": 4}"#, r#"{"sampleCount": 4}"#)]
#[case::zero_count(r#"{"n": 0}"#, r#"{"sampleCount": 0}"#)]
#[case::negative_count(r#"{"n": -1}"#, r#"{"sampleCount": -1}"#)]
#[case::count_above_i64(
    r#"{"n": 9223372036854775808}"#,
    r#"{"sampleCount": 9223372036854775808}"#
)]
#[case::nulls(r#"{"n": null, "size": null}"#, "{}")]
fn media_optional_params_image_vertex_ai_maps_n_and_size(
    #[case] passed: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        generate(passed, Some("vertex_ai")).unwrap(),
        params(expected)
    );
}

#[rstest]
#[case::text_count(r#"{"n": "2"}"#, "n")]
#[case::float_count(r#"{"n": 2.5}"#, "n")]
#[case::bool_count(r#"{"n": true}"#, "n")]
#[case::number_size(r#"{"size": 5}"#, "size")]
#[case::list_size(r#"{"size": ["1x1"]}"#, "size")]
fn media_optional_params_image_vertex_ai_rejects_wrong_types(
    #[case] passed: &str,
    #[case] field: &str,
) {
    let error = generate(passed, Some("vertex_ai")).unwrap_err();
    assert!(matches!(&error, MediaParamsError::WrongType(name, _) if *name == field));
    assert_eq!(error.status_code(), 400);
}

#[test]
fn media_optional_params_image_vertex_ai_rejects_other_params() {
    let error = generate(r#"{"n": 1, "quality": "hd"}"#, Some("vertex_ai")).unwrap_err();
    assert_eq!(
        error.to_string(),
        "quality is not supported by vertex_ai for m"
    );
}

#[test]
fn media_optional_params_image_vertex_ai_drops_unsupported_params_with_the_policy() {
    let passed = params(r#"{"n": 1, "quality": "hd", "top_k": 3}"#);
    let vertex = ImageGenerationRequest {
        drop_policy: DROP_ALL,
        ..request(&passed, Some("vertex_ai"))
    };
    let result = get_optional_params_image_gen(&vertex).unwrap();
    assert_eq!(result, params(r#"{"sampleCount": 1, "top_k": 3}"#));
}

#[test]
fn media_optional_params_image_dropped_param_does_not_travel_as_an_extra() {
    let passed = params(r#"{"n": 1, "foo": "f", "quality": "hd"}"#);
    let config = ImageGenerationRequest {
        provider_config: Some(&CONFIG),
        drop_policy: DROP_ALL,
        ..request(&passed, Some("other"))
    };
    let result = get_optional_params_image_gen(&config).unwrap();
    assert_eq!(
        result,
        params(r#"{"n": 1, "foo": "f", "seen_drop": true, "seen_model": "m"}"#)
    );
}
