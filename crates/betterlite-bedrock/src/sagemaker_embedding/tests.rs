#![cfg(test)]

use std::fs;
use std::io;
use std::path::Path;

use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_prompts::arguments::json::Object;
use betterlite_tokenizer::engine::error::EngineError;
use rstest::rstest;

use crate::sagemaker_embedding::cohere::transform_response as cohere_reply;
use crate::sagemaker_embedding::hf::transform_response as hf_reply;
use crate::sagemaker_embedding::{
    EmbeddingConfig, VOYAGE_SUPPORTED_OPENAI_PARAMS, get_model_config, supported_openai_params,
    validate_environment,
};

struct Chars;

impl TextEncoding for Chars {
    fn count_tokens(&self, text: &str) -> Result<usize, EngineError> {
        Ok(text.chars().count())
    }
}

// the embedding fuzz target feeds every seed to both response readers
fn check_embedding_seed(name: &str, bytes: &[u8]) {
    let hf = hf_reply(bytes, 200, "meta-llama/Llama-2-7b", &Object::default());
    let cohere = cohere_reply(
        bytes,
        "cohere.embed-english-v3",
        None,
        &Object::default(),
        &Chars,
    );
    match name {
        "hf-list" => assert_eq!(hf.unwrap().data.len(), 2, "{name}"),
        "hf-dict" => assert_eq!(hf.unwrap().data.len(), 1, "{name}"),
        "hf-unexpected-dict" => assert_eq!(hf.unwrap_err().status_code(), 500, "{name}"),
        "cohere-reply" => assert_eq!(cohere.unwrap().usage.prompt_tokens, 3, "{name}"),
        "truncated" | "empty" | "not-json" => {
            assert!(hf.is_err(), "{name} fails the HF reader");
            assert!(cohere.is_err(), "{name} fails the Cohere reader");
        }
        other => panic!("unexpected seed {other}"),
    }
}

#[test]
fn replays_the_sagemaker_embedding_reply_corpus() {
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/sagemaker_embedding_reply");
    let mut seen = 0_u32;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            check_embedding_seed(&name, &bytes);
            seen = seen.saturating_add(1);
            Ok::<(), io::Error>(())
        })
        .unwrap();
    assert!(seen >= 7, "expected committed seeds, found {seen}");
}

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
