#![cfg(test)]

use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_openai::cohere_legacy_embedding::error::CohereLegacyEmbeddingError;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_tokenizer::engine::error::EngineError;
use rstest::rstest;

use crate::sagemaker_embedding::cohere::{
    CohereReplyError, CohereRequestError, map_openai_params, transform_request, transform_response,
};

struct Words;

impl TextEncoding for Words {
    fn count_tokens(&self, text: &str) -> Result<usize, EngineError> {
        Ok(text.split_whitespace().count())
    }
}

fn input(text: &str) -> EmbeddingInput {
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
        &input(r#""hello""#),
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
        &input(r#"["a", "b"]"#),
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
    let built = transform_request("cohere.embed-english-v3", &input("[]"), &Object::new()).unwrap();

    assert_eq!(
        built,
        object(r#"{"input_type": "search_document", "texts": []}"#)
    );
}

#[rstest]
#[case::tokens("[1, 2]")]
#[case::token_lists("[[1, 2]]")]
fn a_token_input_is_refused(#[case] value: &str) {
    let error =
        transform_request("cohere.embed-english-v3", &input(value), &Object::new()).unwrap_err();

    assert_eq!(error, CohereRequestError::InputNotText, "{value}");
    assert_eq!(error.to_string(), "Input must be a list of strings");
}

#[test]
fn the_mapped_options_reach_the_body() {
    let mapped = map_openai_params(
        &object(
            r#"{"encoding_format": "base64", "dimensions": 128, "input_type": "search_query"}"#,
        ),
        Object::new(),
    );
    let built =
        transform_request("cohere.embed-english-v3", &input(r#""hello""#), &mapped).unwrap();

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
        Some(&input(r#""one two three""#)),
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
        Some(&input(r#"["one two three", "four"]"#)),
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
        Some(&input("[]")),
        &object(r#"{"texts": ["one two", "three"]}"#),
        &Words,
    )
    .unwrap();

    assert_eq!(reply.usage.prompt_tokens, 3);
}

#[test]
fn an_empty_text_logged_input_falls_to_the_body_texts() {
    let reply = transform_response(
        br#"{"embeddings": [[0.5]]}"#,
        "cohere.embed-english-v3",
        Some(&input(r#""""#)),
        &object(r#"{"texts": ["one two"]}"#),
        &Words,
    )
    .unwrap();

    assert_eq!(reply.usage.prompt_tokens, 2);
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
fn body_texts_win_over_the_body_images() {
    let reply = transform_response(
        br#"{"embeddings": [[0.5]]}"#,
        "cohere.embed-english-v3",
        None,
        &object(r#"{"texts": ["one two three"], "images": ["aGVsbG8="]}"#),
        &Words,
    )
    .unwrap();

    assert_eq!(reply.usage.prompt_tokens, 3);
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
        Some(&input(r#""one two""#)),
        &object(r#"{"texts": ["a", "b", "c"]}"#),
        &Words,
    )
    .unwrap();

    assert_eq!(reply.usage.prompt_tokens, 2);
}

#[rstest]
#[case::tokens("[1]")]
#[case::token_lists("[[1, 2]]")]
fn a_token_logged_input_is_refused(#[case] value: &str) {
    let error = transform_response(
        br#"{"embeddings": [[0.5]]}"#,
        "cohere.embed-english-v3",
        Some(&input(value)),
        &Object::new(),
        &Words,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        CohereReplyError::Legacy(CohereLegacyEmbeddingError::Usage(_))
    ));
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

    assert!(matches!(error, CohereReplyError::Body { .. }));
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

    assert!(matches!(
        error,
        CohereReplyError::Legacy(CohereLegacyEmbeddingError::Embeddings)
    ));
}

#[rstest]
#[case::bool(b"true")]
#[case::number(b"5")]
#[case::text(b"\"hello\"")]
#[case::list(b"[]")]
#[case::null(b"null")]
fn a_body_that_is_not_an_object_is_refused_as_embeddingless(#[case] body: &[u8]) {
    let error = transform_response(
        body,
        "cohere.embed-english-v3",
        None,
        &Object::new(),
        &Words,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        CohereReplyError::Legacy(CohereLegacyEmbeddingError::Embeddings)
    ));
}

// the request value becomes an embedding input only once chosen, and its serde error stays as the cause
#[test]
fn a_body_texts_value_that_is_not_an_input_keeps_the_serde_error() {
    let error = transform_response(
        br#"{"embeddings": [[0.5]]}"#,
        "cohere.embed-english-v3",
        None,
        &object(r#"{"texts": [true]}"#),
        &Words,
    )
    .unwrap_err();

    let CohereReplyError::Request { source } = error else {
        panic!("expected a request error");
    };
    assert!(source.is_data(), "{source}");
}

#[test]
fn the_replied_rows_keep_their_numbers() {
    let reply = transform_response(
        br#"{"embeddings": [[0.5, 0.25], [1.5]]}"#,
        "cohere.embed-english-v3",
        None,
        &Object::new(),
        &Words,
    )
    .unwrap();

    assert_eq!(
        serde_json::from_value::<Json>(serde_json::to_value(&reply.data).unwrap()).unwrap(),
        serde_json::from_str::<Json>(
            r#"[{"object": "embedding", "index": 0, "embedding": [0.5, 0.25]},
                {"object": "embedding", "index": 1, "embedding": [1.5]}]"#
        )
        .unwrap()
    );
}

#[test]
fn a_base64_reply_row_is_kept_as_text() {
    let reply = transform_response(
        br#"{"embeddings": ["aGVsbG8="]}"#,
        "cohere.embed-english-v3",
        None,
        &Object::new(),
        &Words,
    )
    .unwrap();

    assert_eq!(
        serde_json::from_value::<Json>(serde_json::to_value(&reply.data).unwrap()).unwrap(),
        serde_json::from_str::<Json>(
            r#"[{"object": "embedding", "index": 0, "embedding": "aGVsbG8="}]"#
        )
        .unwrap()
    );
}

#[test]
fn a_grouped_reply_labels_every_row_with_its_type() {
    let reply = transform_response(
        br#"{"embeddings": {"float": [[0.5]], "int8": [[3.5], [4.5]]}}"#,
        "cohere.embed-english-v3",
        None,
        &Object::new(),
        &Words,
    )
    .unwrap();

    assert_eq!(
        serde_json::from_value::<Json>(serde_json::to_value(&reply.data).unwrap()).unwrap(),
        serde_json::from_str::<Json>(
            r#"[{"object": "embedding", "index": 0, "embedding": [0.5], "type": "float"},
                {"object": "embedding", "index": 0, "embedding": [3.5], "type": "int8"},
                {"object": "embedding", "index": 1, "embedding": [4.5], "type": "int8"}]"#
        )
        .unwrap()
    );
}

#[test]
fn a_rows_reply_marked_by_type_is_refused_as_embeddingless() {
    let error = transform_response(
        br#"{"embeddings": [[0.5]], "response_type": "embeddings_by_type"}"#,
        "cohere.embed-english-v3",
        None,
        &Object::new(),
        &Words,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        CohereReplyError::Legacy(CohereLegacyEmbeddingError::Embeddings)
    ));
}

#[test]
fn image_billed_units_count_as_prompt_tokens() {
    let reply = transform_response(
        br#"{"embeddings": [[0.5]], "meta": {"billed_units": {"images": 2}}}"#,
        "cohere.embed-english-v3",
        None,
        &Object::new(),
        &Words,
    )
    .unwrap();

    assert_eq!(reply.usage.prompt_tokens, 2);
    assert_eq!(reply.usage.total_tokens, 2);
    assert_eq!(reply.usage.completion_tokens, 0);
    let details = reply.usage.prompt_tokens_details.as_ref().unwrap();
    assert_eq!(details.text_tokens, None);
    assert_eq!(details.image_tokens, Some(2));
}

#[rstest]
#[case::row(br#"{"embeddings": [[true]]}"#)]
#[case::group(br#"{"embeddings": {"float": 5}}"#)]
#[case::response_type(br#"{"embeddings": [[0.5]], "response_type": 5}"#)]
#[case::meta(br#"{"embeddings": [[0.5]], "meta": {"billed_units": {"input_tokens": "seven"}}}"#)]
fn a_reply_outside_the_declared_shape_is_refused(#[case] body: &[u8]) {
    let error = transform_response(
        body,
        "cohere.embed-english-v3",
        None,
        &Object::new(),
        &Words,
    )
    .unwrap_err();

    assert!(matches!(error, CohereReplyError::Shape { .. }), "{error}");
}
