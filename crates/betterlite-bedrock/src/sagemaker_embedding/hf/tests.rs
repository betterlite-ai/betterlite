#![cfg(test)]

use betterlite_openai::embedding::{EmbeddingInput, EmbeddingVector};
use betterlite_prompts::arguments::json::Object;

use crate::sagemaker_embedding::hf::{
    HfEmbeddingRecord, HfUsage, transform_request, transform_response,
};

fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn input(text: &str) -> EmbeddingInput {
    serde_json::from_str(text).unwrap()
}

fn vector(text: &str) -> EmbeddingVector {
    serde_json::from_str(text).unwrap()
}

fn inputs(text: &str) -> Object {
    object(&format!(r#"{{"inputs": {text}}}"#))
}

#[test]
fn request_is_the_inputs_key_and_the_options() {
    let built = transform_request(&input(r#""hello""#), &object(r#"{"truncate": true}"#));

    assert_eq!(built, object(r#"{"inputs": "hello", "truncate": true}"#));
    assert_eq!(
        built.keys().map(String::as_str).collect::<Vec<_>>(),
        ["inputs", "truncate"]
    );
}

#[test]
fn an_inputs_option_replaces_the_input() {
    let built = transform_request(
        &input(r#""hello""#),
        &object(r#"{"inputs": ["kept"], "truncate": false}"#),
    );

    assert_eq!(built, object(r#"{"inputs": ["kept"], "truncate": false}"#));
    assert_eq!(
        built.get("inputs"),
        Some(&object(r#"{"inputs": ["kept"]}"#)["inputs"]),
        "the option wins over the request input"
    );
}

#[test]
fn token_inputs_are_sent_as_they_came() {
    let built = transform_request(&input(r#"[[2, 3]]"#), &Object::new());

    assert_eq!(built, object(r#"{"inputs": [[2, 3]]}"#));
}

#[test]
fn a_raw_list_reply_becomes_embedding_rows() {
    let reply = transform_response(
        br#"[[0.1, 0.2], [0.3]]"#,
        200,
        "meta-llama/Llama-2-7b",
        &inputs(r#"["hello world"]"#),
    )
    .unwrap();

    assert_eq!(reply.object, "list");
    assert_eq!(reply.model, "meta-llama/Llama-2-7b");
    assert_eq!(
        reply.data,
        [
            HfEmbeddingRecord {
                object: "embedding",
                index: 0,
                embedding: vector("[0.1, 0.2]"),
            },
            HfEmbeddingRecord {
                object: "embedding",
                index: 1,
                embedding: vector("[0.3]"),
            },
        ]
    );
    assert_eq!(
        reply.usage,
        HfUsage {
            prompt_tokens: 2,
            completion_tokens: 0,
            total_tokens: 2,
        }
    );
}

#[test]
fn a_wrapped_dict_reply_reads_its_embedding_field() {
    let reply = transform_response(
        br#"{"embedding": [[1, 2]]}"#,
        200,
        "model",
        &inputs(r#"["one two three"]"#),
    )
    .unwrap();

    assert_eq!(
        reply.data,
        [HfEmbeddingRecord {
            object: "embedding",
            index: 0,
            embedding: vector("[1, 2]"),
        }]
    );
    assert_eq!(reply.usage.prompt_tokens, 3);
}

#[test]
fn every_string_in_the_inputs_adds_its_words() {
    let reply = transform_response(
        br#"[]"#,
        200,
        "model",
        &inputs(r#"["one two", " three ", ""]"#),
    )
    .unwrap();

    assert_eq!(reply.usage.prompt_tokens, 3);
    assert_eq!(reply.usage.total_tokens, 3);
    assert_eq!(reply.usage.completion_tokens, 0);
}

#[test]
fn a_text_input_counts_its_non_space_characters() {
    let reply = transform_response(br#"[]"#, 200, "model", &inputs(r#""hello world""#)).unwrap();

    assert_eq!(reply.usage.prompt_tokens, 10);
}

#[test]
fn a_token_input_adds_no_words() {
    let reply = transform_response(br#"[]"#, 200, "model", &inputs("[1, 2]")).unwrap();

    assert_eq!(reply.usage.prompt_tokens, 0);
}

#[test]
fn a_token_list_input_adds_no_words() {
    let reply = transform_response(br#"[]"#, 200, "model", &inputs("[[1, 2]]")).unwrap();

    assert_eq!(reply.usage.prompt_tokens, 0);
}

#[test]
fn an_absent_inputs_key_counts_no_words() {
    let reply = transform_response(br#"[]"#, 200, "model", &Object::new()).unwrap();

    assert_eq!(
        reply.usage,
        HfUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
        }
    );
}

#[test]
fn an_inputs_option_changes_the_counted_words() {
    let request = transform_request(
        &input(r#""one two three""#),
        &object(r#"{"inputs": ["one two"]}"#),
    );
    let reply = transform_response(br#"[]"#, 200, "model", &request).unwrap();

    assert_eq!(reply.usage.prompt_tokens, 2);
}

#[test]
fn an_unparsable_reply_carries_the_http_status() {
    let error = transform_response(b"not json", 503, "model", &Object::new()).unwrap_err();

    assert_eq!(error.status_code(), 503);
    assert!(error.to_string().starts_with("Failed to parse response: "));
}

#[test]
fn a_dict_without_an_embedding_key_names_its_type() {
    let error = transform_response(br#"{"data": []}"#, 200, "model", &Object::new()).unwrap_err();

    assert_eq!(error.status_code(), 500);
    assert_eq!(
        error.to_string(),
        "Unexpected response format. Expected list or dict with 'embedding' key, got: dict"
    );
}

#[test]
fn a_text_reply_names_its_python_type() {
    let error = transform_response(br#""text""#, 200, "model", &Object::new()).unwrap_err();

    assert_eq!(
        error.to_string(),
        "Unexpected response format. Expected list or dict with 'embedding' key, got: str"
    );
}

#[test]
fn a_null_reply_names_its_python_type() {
    let error = transform_response(b"null", 200, "model", &Object::new()).unwrap_err();

    assert_eq!(
        error.to_string(),
        "Unexpected response format. Expected list or dict with 'embedding' key, got: NoneType"
    );
}

#[test]
fn an_integer_reply_names_its_python_type() {
    let error = transform_response(b"7", 200, "model", &Object::new()).unwrap_err();

    assert_eq!(
        error.to_string(),
        "Unexpected response format. Expected list or dict with 'embedding' key, got: int"
    );
}

// a negative number is an int only through is_i64, so the || guard cannot become &&
#[test]
fn a_negative_integer_reply_names_its_python_type() {
    let error = transform_response(b"-7", 200, "model", &Object::new()).unwrap_err();

    assert_eq!(
        error.to_string(),
        "Unexpected response format. Expected list or dict with 'embedding' key, got: int"
    );
}

#[test]
fn a_float_reply_names_its_python_type() {
    let error = transform_response(b"7.5", 200, "model", &Object::new()).unwrap_err();

    assert_eq!(
        error.to_string(),
        "Unexpected response format. Expected list or dict with 'embedding' key, got: float"
    );
}

#[test]
fn a_boolean_reply_names_its_python_type() {
    let error = transform_response(b"true", 200, "model", &Object::new()).unwrap_err();

    assert_eq!(
        error.to_string(),
        "Unexpected response format. Expected list or dict with 'embedding' key, got: bool"
    );
}

#[test]
fn an_embedding_that_is_not_a_list_is_unexpected() {
    let error =
        transform_response(br#"{"embedding": "AAAA"}"#, 200, "model", &Object::new()).unwrap_err();

    assert_eq!(error.status_code(), 422);
    assert_eq!(
        error.to_string(),
        "HF response not in expected format - \"AAAA\""
    );
}

#[test]
fn a_null_embedding_is_not_a_list() {
    let error =
        transform_response(br#"{"embedding": null}"#, 200, "model", &Object::new()).unwrap_err();

    assert_eq!(error.status_code(), 422);
    assert_eq!(
        error.to_string(),
        "HF response not in expected format - null"
    );
}

// LiteLLM stores such a row unchecked and answers 200; the shared EmbeddingVector cannot hold it
#[test]
fn a_row_that_is_not_numbers_or_text_is_refused() {
    let error = transform_response(br#"{"embedding": [[true]]}"#, 200, "model", &Object::new())
        .unwrap_err();

    assert_eq!(error.status_code(), 422);
    assert_eq!(
        error.to_string(),
        "HF response not in expected format - [[true]]"
    );
}

#[test]
fn an_inputs_value_that_is_not_text_or_a_list_is_named() {
    let error = transform_response(br#"[]"#, 200, "model", &inputs("5")).unwrap_err();

    assert_eq!(error.status_code(), 500);
    assert_eq!(
        error.to_string(),
        "the HF request inputs are neither a text nor a list of texts"
    );
}
