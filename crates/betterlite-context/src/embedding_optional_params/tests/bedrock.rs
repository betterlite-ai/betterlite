#![cfg(test)]

use rstest::rstest;

use super::{FixtureMapper, base, params, parse_pair};
use crate::embedding_optional_params::bedrock::{BedrockEmbeddingFamily, bedrock_embedding_family};
use crate::embedding_optional_params::embedding_optional_params;
use crate::embedding_optional_params::error::EmbeddingParamsError;
use crate::non_default_params::Params;
use crate::non_default_params::mapper::DropPolicy;

#[rstest]
#[case::titan_text_v1("amazon.titan-embed-text-v1", BedrockEmbeddingFamily::TitanTextV1)]
#[case::titan_image_v1("amazon.titan-embed-image-v1", BedrockEmbeddingFamily::TitanImageV1)]
#[case::titan_text_v2("amazon.titan-embed-text-v2:0", BedrockEmbeddingFamily::TitanTextV2)]
#[case::cohere("cohere.embed-english-v3", BedrockEmbeddingFamily::CohereEmbed)]
#[case::twelvelabs(
    "twelvelabs.marengo-embed-2-7-v1:0",
    BedrockEmbeddingFamily::TwelveLabsMarengo
)]
#[case::marengo("marengo-embed-3", BedrockEmbeddingFamily::TwelveLabsMarengo)]
#[case::nova(
    "amazon.nova-2-multimodal-embeddings-v1:0",
    BedrockEmbeddingFamily::Nova
)]
#[case::nova_upper("NOVA-model", BedrockEmbeddingFamily::Nova)]
#[case::unmapped("mystery-model", BedrockEmbeddingFamily::Unmapped)]
#[case::text_v1_before_image(
    "amazon.titan-embed-text-v1-amazon.titan-embed-image-v1",
    BedrockEmbeddingFamily::TitanTextV1
)]
#[case::image_before_v2(
    "amazon.titan-embed-image-v1-amazon.titan-embed-text-v2:0",
    BedrockEmbeddingFamily::TitanImageV1
)]
#[case::cohere_before_nova("cohere.embed-nova", BedrockEmbeddingFamily::CohereEmbed)]
#[case::twelvelabs_before_nova("twelvelabs-nova", BedrockEmbeddingFamily::TwelveLabsMarengo)]
fn bedrock_embedding_family_resolves_in_source_order(
    #[case] model: &str,
    #[case] family: BedrockEmbeddingFamily,
) {
    assert_eq!(bedrock_embedding_family(model), family);
}

#[rstest]
#[case::titan_text_v1("amazon.titan-embed-text-v1", &[], "{}", "{}")]
#[case::titan_image_v1(
    "amazon.titan-embed-image-v1",
    &["dimensions"],
    r#"{"dimensions": 2}"#,
    r#"{"embeddingConfig": {"outputEmbeddingLength": 2}}"#
)]
#[case::titan_text_v2(
    "amazon.titan-embed-text-v2:0",
    &["dimensions", "encoding_format"],
    r#"{"dimensions": 2, "encoding_format": "float"}"#,
    r#"{"dimensions": 2, "embeddingTypes": ["float"]}"#
)]
#[case::cohere(
    "cohere.embed-english-v3",
    &["encoding_format", "dimensions"],
    r#"{"dimensions": 2, "encoding_format": "float"}"#,
    r#"{"embedding_types": ["float"], "output_dimension": 2}"#
)]
#[case::twelvelabs(
    "twelvelabs.marengo-embed-2-7-v1:0",
    &["dimensions"],
    r#"{"dimensions": 2}"#,
    r#"{"embedding_dimension": 2}"#
)]
#[case::nova(
    "amazon.nova-2-multimodal-embeddings-v1:0",
    &["dimensions"],
    r#"{"dimensions": 2}"#,
    r#"{"embedding_dimension": 2}"#
)]
fn bedrock_families_map_through_the_provider_config(
    #[case] model: &str,
    #[case] supported: &[&str],
    #[case] passed_json: &str,
    #[case] mapped: &str,
) {
    let (passed, special) = parse_pair(passed_json, "{}");
    let fixture = FixtureMapper::mapper(supported, mapped);
    let mut request = base(model, "bedrock", &passed, &special);
    request.mapper = Some(&fixture);
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, params(mapped));
}

#[test]
fn bedrock_family_rejects_a_param_outside_its_supported_list() {
    let (passed, special) = parse_pair(r#"{"dimensions": 2}"#, "{}");
    let fixture = FixtureMapper::mapper(&[], "{}");
    let mut request = base("amazon.titan-embed-text-v1", "bedrock", &passed, &special);
    request.mapper = Some(&fixture);
    let error = embedding_optional_params(&request).unwrap_err();
    assert!(matches!(&error, EmbeddingParamsError::Unsupported { .. }));
    assert_eq!(error.status_code(), 400);
}

#[test]
fn bedrock_family_maps_with_the_per_call_drop_and_the_model() {
    let (passed, special) = parse_pair(r#"{"dimensions": 2}"#, "{}");
    let fixture = FixtureMapper::mapper(&["dimensions"], "{}");
    let mut request = base("amazon.titan-embed-text-v2:0", "bedrock", &passed, &special);
    request.mapper = Some(&fixture);
    request.drop_policy = DropPolicy {
        drop_params: Some(true),
        litellm_drop_params: false,
    };
    embedding_optional_params(&request).unwrap();
    let seen = fixture.seen();
    assert_eq!(seen[0].model, "amazon.titan-embed-text-v2:0");
    assert!(seen[0].drop_params);
}

#[test]
fn bedrock_unmapped_returns_the_special_params() {
    let (passed, special) = parse_pair(r#"{"custom_field": 1}"#, r#"{"aws_region_name": "r"}"#);
    let request = base("mystery-model", "bedrock", &passed, &special);
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, params(r#"{"aws_region_name": "r"}"#));
}

#[test]
fn bedrock_unmapped_keeps_an_empty_extra_body() {
    let (passed, special) = parse_pair("{}", r#"{"extra_body": {}}"#);
    let request = base("mystery-model", "bedrock", &passed, &special);
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, params(r#"{"extra_body": {}}"#));
}

#[test]
fn bedrock_unmapped_rejects_a_non_default_param() {
    let (passed, special) = parse_pair(r#"{"dimensions": 2}"#, "{}");
    let request = base("mystery-model", "bedrock", &passed, &special);
    let error = embedding_optional_params(&request).unwrap_err();
    assert!(matches!(&error, EmbeddingParamsError::Unsupported { .. }));
    assert_eq!(error.status_code(), 400);
}

#[test]
fn bedrock_unmapped_drops_the_non_default_param_with_drop_params() {
    let (passed, special) = parse_pair(r#"{"dimensions": 2}"#, r#"{"extra_body": {}}"#);
    let mut request = base("mystery-model", "bedrock", &passed, &special);
    request.drop_policy = DropPolicy {
        drop_params: Some(true),
        litellm_drop_params: false,
    };
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, params(r#"{"extra_body": {}}"#));
}

#[test]
fn bedrock_unmapped_ignores_a_provided_mapper() {
    let (passed, special) = parse_pair("{}", "{}");
    let fixture = FixtureMapper::mapper(&["dimensions"], r#"{"mapped": true}"#);
    let mut request = base("mystery-model", "bedrock", &passed, &special);
    request.mapper = Some(&fixture);
    let result = embedding_optional_params(&request).unwrap();
    assert_eq!(result, Params::new());
    assert!(fixture.seen().is_empty());
}
