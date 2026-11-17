#![cfg(test)]

use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_openai::cohere_legacy_embedding::error::CohereLegacyEmbeddingError;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_tokenizer::engine::error::EngineError;

use crate::sagemaker_embedding::cohere::{
    CohereRequestError, map_openai_params, transform_request, transform_response,
};

struct Words;

impl TextEncoding for Words {
    fn count_tokens(&self, text: &str) -> Result<usize, EngineError> {
        Ok(text.split_whitespace().count())
    }
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

#[test]
fn the_supported_list_adds_input_type_to_the_bedrock_one() {
    assert_eq!(
        super::SUPPORTED_OPENAI_PARAMS,
        ["encoding_format", "dimensions", "input_type"].as_slice()
    );
}

#[test]
fn mapping_goes_through_the_bedrock_mapper() {
    let mapped = map_openai_params(
        &object(r#"{"encoding_format": ["base64", "float"], "dimensions": 256, "user": "u"}"#),
        Object::new(),
    );

    assert_eq!(
        mapped,
        object(r#"{"embedding_types": ["float", "float"], "output_dimension": 256}"#)
    );
}

#[test]
fn input_type_is_copied_after_the_bedrock_mapping() {
    let mapped = map_openai_params(
        &object(r#"{"input_type": "search_query", "dimensions": 512}"#),
        object(r#"{"input_type": "classification"}"#),
    );

    assert_eq!(
        mapped,
        object(r#"{"output_dimension": 512, "input_type": "search_query"}"#)
    );
}

#[test]
fn an_absent_input_type_keeps_the_callers_optional_params() {
    let mapped = map_openai_params(
        &object(r#"{"dimensions": 256}"#),
        object(r#"{"input_type": "classification"}"#),
    );

    assert_eq!(
        mapped,
        object(r#"{"output_dimension": 256, "input_type": "classification"}"#)
    );
}

#[test]
fn a_text_input_becomes_one_text() {
    let built = transform_request(
        "cohere.embed-english-v3",
        &json(r#""hello""#),
        &Object::new(),
    )
    .unwrap();

    assert_eq!(
        built,
        object(r#"{"input_type": "search_document", "texts": ["hello"]}"#)
    );
}

#[test]
fn a_list_of_texts_is_kept() {
    let built = transform_request(
        "cohere.embed-english-v3",
        &json(r#"["a", "b"]"#),
        &object(r#"{"truncate": "END"}"#),
    )
    .unwrap();

    assert_eq!(
        built,
        object(r#"{"input_type": "search_document", "texts": ["a", "b"], "truncate": "END"}"#)
    );
}

#[test]
fn an_empty_list_stays_empty() {
    let built = transform_request("cohere.embed-english-v3", &json("[]"), &Object::new()).unwrap();

    assert_eq!(
        built,
        object(r#"{"input_type": "search_document", "texts": []}"#)
    );
}

#[test]
fn a_list_of_integers_is_refused() {
    for input in ["[1, 2]", r#"[["tokens"]]"#, "[true]", "[null]"] {
        let error =
            transform_request("cohere.embed-english-v3", &json(input), &Object::new()).unwrap_err();

        assert_eq!(error, CohereRequestError::InputNotText, "{input}");
        assert_eq!(error.to_string(), "Input must be a list of strings");
    }
}

#[test]
fn a_later_non_text_item_is_refused() {
    let error = transform_request(
        "cohere.embed-english-v3",
        &json(r#"["ok", 2]"#),
        &Object::new(),
    )
    .unwrap_err();

    assert_eq!(error, CohereRequestError::InputNotText);
}

#[test]
fn a_value_that_is_not_text_or_a_list_is_refused() {
    for input in ["7", "true", "null", r#"{"texts": ["a"]}"#] {
        let error =
            transform_request("cohere.embed-english-v3", &json(input), &Object::new()).unwrap_err();

        assert_eq!(error, CohereRequestError::InputNotText, "{input}");
    }
}

#[test]
fn the_mapped_options_reach_the_body() {
    let mapped = map_openai_params(
        &object(
            r#"{"encoding_format": "base64", "dimensions": 128, "input_type": "search_query"}"#,
        ),
        Object::new(),
    );
    let built = transform_request("cohere.embed-english-v3", &json(r#""hello""#), &mapped).unwrap();

    assert_eq!(
        built,
        object(
            r#"{"input_type": "search_query", "texts": ["hello"], "embedding_types": ["float"], "output_dimension": 128}"#
        )
    );
}

#[test]
fn billed_units_win_over_the_counted_input() {
    let reply = transform_response(
        br#"{"embeddings": [[0.5, 0.25]], "meta": {"billed_units": {"input_tokens": 7}}}"#,
        "cohere.embed-english-v3",
        Some(&json(r#""one two three""#)),
        &object(r#"{"texts": ["one two three"]}"#),
        &Words,
    )
    .unwrap();

    assert_eq!(reply.object, "list");
    assert_eq!(reply.model, "cohere.embed-english-v3");
    assert_eq!(reply.usage.prompt_tokens, 7);
    assert_eq!(reply.usage.total_tokens, 7);
    assert_eq!(reply.usage.completion_tokens, 0);
}

#[test]
fn the_logged_input_is_counted_when_the_reply_bills_nothing() {
    let reply = transform_response(
        br#"{"embeddings": [[0.5]]}"#,
        "cohere.embed-english-v3",
        Some(&json(r#"["one two three", "four"]"#)),
        &object(r#"{"texts": ["ignored"]}"#),
        &Words,
    )
    .unwrap();

    assert_eq!(reply.usage.prompt_tokens, 4);
}

#[test]
fn an_empty_logged_input_falls_to_the_body_texts() {
    let reply = transform_response(
        br#"{"embeddings": [[0.5]]}"#,
        "cohere.embed-english-v3",
        Some(&json("[]")),
        &object(r#"{"texts": ["one two", "three"]}"#),
        &Words,
    )
    .unwrap();

    assert_eq!(reply.usage.prompt_tokens, 3);
}

#[test]
fn empty_body_texts_fall_to_the_body_images() {
    let reply = transform_response(
        br#"{"embeddings": [[0.5]]}"#,
        "cohere.embed-english-v3",
        None,
        &object(r#"{"texts": [], "images": ["aGVsbG8="]}"#),
        &Words,
    )
    .unwrap();

    assert_eq!(reply.usage.prompt_tokens, 1);
}

#[test]
fn no_input_anywhere_counts_no_tokens() {
    let reply = transform_response(
        br#"{"embeddings": [[0.5]]}"#,
        "cohere.embed-english-v3",
        None,
        &object(r#"{"texts": []}"#),
        &Words,
    )
    .unwrap();

    assert_eq!(reply.usage.prompt_tokens, 0);
}

#[test]
fn a_text_logged_input_becomes_one_item() {
    let reply = transform_response(
        br#"{"embeddings": [[0.5]]}"#,
        "cohere.embed-english-v3",
        Some(&json(r#""one two""#)),
        &object(r#"{"texts": ["a", "b", "c"]}"#),
        &Words,
    )
    .unwrap();

    assert_eq!(reply.usage.prompt_tokens, 2);
}

#[test]
fn a_non_text_logged_input_is_refused() {
    let error = transform_response(
        br#"{"embeddings": [[0.5]]}"#,
        "cohere.embed-english-v3",
        Some(&json(r#"{"tokens": [1]}"#)),
        &Object::new(),
        &Words,
    )
    .unwrap_err();

    assert!(matches!(error, CohereLegacyEmbeddingError::Usage(_)));
}

#[test]
fn a_body_that_is_not_json_is_refused() {
    let error = transform_response(
        b"not json",
        "cohere.embed-english-v3",
        None,
        &Object::new(),
        &Words,
    )
    .unwrap_err();

    assert!(matches!(error, CohereLegacyEmbeddingError::Json(_)));
}

#[test]
fn a_body_without_embeddings_is_refused() {
    let error = transform_response(
        br#"{"meta": {}}"#,
        "cohere.embed-english-v3",
        None,
        &Object::new(),
        &Words,
    )
    .unwrap_err();

    assert!(matches!(error, CohereLegacyEmbeddingError::Embeddings));
}
