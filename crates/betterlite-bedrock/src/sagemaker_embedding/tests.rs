#![cfg(test)]

use rstest::rstest;

use crate::sagemaker_embedding::{
    EmbeddingConfig, VOYAGE_SUPPORTED_OPENAI_PARAMS, get_model_config, supported_openai_params,
    validate_environment,
};

#[rstest]
#[case::plain_hf("meta-llama/Llama-2-7b", EmbeddingConfig::HuggingFace)]
#[case::cohere("cohere.embed-english-v3", EmbeddingConfig::Cohere)]
#[case::voyage("voyage-3", EmbeddingConfig::Voyage)]
#[case::upper_case("VOYAGE-3", EmbeddingConfig::Voyage)]
#[case::voyage_before_cohere("cohere-voyage-x", EmbeddingConfig::Voyage)]
#[case::cohere_after_voyage_check("cohere.embed-multilingual-v3", EmbeddingConfig::Cohere)]
#[case::voyage_inside_a_longer_name("sagemaker-voyage-large", EmbeddingConfig::Voyage)]
fn model_names_pick_the_litellm_config(#[case] model: &str, #[case] expected: EmbeddingConfig) {
    assert_eq!(get_model_config(model), expected);
}

#[rstest]
#[case::voyage("voyage-3", vec!["encoding_format", "dimensions"])]
#[case::cohere(
    "cohere.embed-english-v3",
    vec!["encoding_format", "dimensions", "input_type"]
)]
#[case::hugging_face("meta-llama/Llama-2-7b", Vec::new())]
fn supported_options_follow_the_config(#[case] model: &str, #[case] expected: Vec<&str>) {
    assert_eq!(supported_openai_params(model), expected.as_slice());
}

#[test]
fn voyage_options_are_the_voyage_config_list() {
    assert_eq!(
        VOYAGE_SUPPORTED_OPENAI_PARAMS,
        ["encoding_format", "dimensions"].as_slice()
    );
}

#[test]
fn validate_environment_keeps_only_the_content_type() {
    let headers = validate_environment();

    assert_eq!(
        headers.into_iter().collect::<Vec<_>>(),
        [(
            String::from("Content-Type"),
            String::from("application/json")
        )]
    );
}
