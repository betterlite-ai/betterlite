use betterlite_openai::embedding::EmbeddingResponse;
use rstest::rstest;

use super::support::json;
use super::support::vector;
use crate::vertex_text_embeddings::error::VertexTextEmbeddingError;
use crate::vertex_text_embeddings::response::transform_response;

fn response(value: &str, model: &str) -> EmbeddingResponse {
    transform_response(&json(value), model).unwrap()
}

#[test]
fn standard_answers_sum_each_token_count() {
    let value = r#"{"predictions": [
        {"embeddings": {"values": [0.1, 0.2], "statistics": {"token_count": 3, "truncated": false}}},
        {"embeddings": {"values": [0.3], "statistics": {"token_count": 4}}}
    ]}"#;
    let answer = response(value, "text-embedding-005");
    assert_eq!(answer.object, "list");
    assert_eq!(answer.model, "text-embedding-005");
    assert_eq!(answer.data.len(), 2);
    assert_eq!(answer.data[0].object, "embedding");
    assert_eq!(answer.data[0].index, 0);
    assert_eq!(answer.data[0].embedding, vector("[0.1, 0.2]"));
    assert_eq!(answer.data[1].index, 1);
    assert_eq!(answer.data[1].embedding, vector("[0.3]"));
    assert_eq!(answer.usage.prompt_tokens, 7);
    assert_eq!(answer.usage.completion_tokens, 0);
    assert_eq!(answer.usage.total_tokens, 7);
    assert_eq!(answer.usage.completion_tokens_details, None);
    assert_eq!(answer.usage.prompt_tokens_details, None);
}

#[test]
fn standard_answers_without_predictions_report_zero_usage() {
    let answer = response(r#"{"predictions": []}"#, "text-embedding-005");
    assert!(answer.data.is_empty());
    assert_eq!(answer.object, "list");
    assert_eq!(answer.usage.prompt_tokens, 0);
    assert_eq!(answer.usage.total_tokens, 0);
}

#[test]
fn result_indices_follow_prediction_order() {
    let value = r#"{"predictions": [
        {"embeddings": {"values": [0.1], "statistics": {"token_count": 1}, "index": 7}},
        {"embeddings": {"values": [0.2], "statistics": {"token_count": 1}, "index": 0}}
    ]}"#;
    let answer = response(value, "text-embedding-005");
    assert_eq!(answer.data[0].index, 0);
    assert_eq!(answer.data[1].index, 1);
}

#[test]
fn tuned_answers_unwrap_the_first_nested_vector() {
    let value = r#"{"predictions": [[[0.1, 0.2]], [[0.3]]]}"#;
    let answer = response(value, "123");
    assert_eq!(answer.object, "list");
    assert_eq!(answer.data.len(), 2);
    assert_eq!(answer.data[0].embedding, vector("[0.1, 0.2]"));
    assert_eq!(answer.data[1].embedding, vector("[0.3]"));
    assert_eq!(answer.usage.prompt_tokens, 0);
    assert_eq!(answer.usage.completion_tokens, 0);
    assert_eq!(answer.usage.total_tokens, 0);
}

#[test]
fn bge_answers_keep_their_vectors_unwrapped() {
    let value = r#"{"predictions": [[0.1, 0.2], [0.3]]}"#;
    let answer = response(value, "bge-small-en-v1.5");
    assert_eq!(answer.object, "list");
    assert_eq!(answer.data.len(), 2);
    assert_eq!(answer.data[0].object, "embedding");
    assert_eq!(answer.data[0].embedding, vector("[0.1, 0.2]"));
    assert_eq!(answer.data[1].embedding, vector("[0.3]"));
    assert_eq!(answer.usage.prompt_tokens, 0);
    assert_eq!(answer.usage.total_tokens, 0);
}

#[rstest]
#[case::missing_predictions("{}", VertexTextEmbeddingError::MissingPredictions)]
#[case::predictions_not_list(
    r#"{"predictions": "no"}"#,
    VertexTextEmbeddingError::PredictionsNotList
)]
#[case::vector_not_list(
    r#"{"predictions": [[0.1], "no"]}"#,
    VertexTextEmbeddingError::NotNumbers { field: "embedding" }
)]
#[case::vector_not_numbers(
    r#"{"predictions": [[0.1], [true]]}"#,
    VertexTextEmbeddingError::NotNumbers { field: "embedding" }
)]
fn bge_answers_require_list_vectors(
    #[case] value: &str,
    #[case] expected: VertexTextEmbeddingError,
) {
    let error = transform_response(&json(value), "bge-small").unwrap_err();
    assert_eq!(error, expected);
}

#[test]
fn bge_rejects_a_non_object_answer() {
    let error = transform_response(&json("[]"), "bge-small").unwrap_err();
    assert_eq!(error, VertexTextEmbeddingError::MissingPredictions);
}

#[rstest]
#[case::outer_not_list(r#"{"predictions": ["no"]}"#, VertexTextEmbeddingError::PredictionNotList { index: 0 })]
#[case::outer_empty(r#"{"predictions": [[]]}"#, VertexTextEmbeddingError::MissingVector { index: 0 })]
#[case::vector_not_numbers(
    r#"{"predictions": [[["no"]]]}"#,
    VertexTextEmbeddingError::NotNumbers { field: "embedding" }
)]
fn tuned_answers_require_one_nested_vector(
    #[case] value: &str,
    #[case] expected: VertexTextEmbeddingError,
) {
    let error = transform_response(&json(value), "42").unwrap_err();
    assert_eq!(error, expected);
}

#[rstest]
#[case::predictions_not_list(
    r#"{"predictions": {}}"#,
    VertexTextEmbeddingError::PredictionsNotList
)]
#[case::element_not_object(r#"{"predictions": [[0.1]]}"#, VertexTextEmbeddingError::MissingField { field: "embeddings" })]
#[case::embeddings_not_object(
    r#"{"predictions": [{"embeddings": []}]}"#,
    VertexTextEmbeddingError::MissingField { field: "values" }
)]
#[case::values_not_list(
    r#"{"predictions": [{"embeddings": {"values": "no", "statistics": {"token_count": 1}}}]}"#,
    VertexTextEmbeddingError::NotNumbers { field: "values" }
)]
#[case::values_not_numbers(
    r#"{"predictions": [{"embeddings": {"values": [true], "statistics": {"token_count": 1}}}]}"#,
    VertexTextEmbeddingError::NotNumbers { field: "values" }
)]
#[case::statistics_missing(
    r#"{"predictions": [{"embeddings": {"values": [0.1]}}]}"#,
    VertexTextEmbeddingError::MissingField { field: "statistics" }
)]
#[case::token_count_missing(
    r#"{"predictions": [{"embeddings": {"values": [0.1], "statistics": {}}}]}"#,
    VertexTextEmbeddingError::MissingField { field: "token_count" }
)]
#[case::token_count_not_number(
    r#"{"predictions": [{"embeddings": {"values": [0.1], "statistics": {"token_count": "3"}}}]}"#,
    VertexTextEmbeddingError::NotNumber { field: "token_count" }
)]
#[case::token_count_fractional(
    r#"{"predictions": [{"embeddings": {"values": [0.1], "statistics": {"token_count": 1.5}}}]}"#,
    VertexTextEmbeddingError::TokenCount
)]
#[case::token_count_negative(
    r#"{"predictions": [{"embeddings": {"values": [0.1], "statistics": {"token_count": -1}}}]}"#,
    VertexTextEmbeddingError::TokenCount
)]
#[case::token_count_too_big(
    r#"{"predictions": [{"embeddings": {"values": [0.1], "statistics": {"token_count": 4294967296}}}]}"#,
    VertexTextEmbeddingError::TokenCount
)]
fn standard_answers_reject_wrong_shapes(
    #[case] value: &str,
    #[case] expected: VertexTextEmbeddingError,
) {
    let error = transform_response(&json(value), "text-embedding-005").unwrap_err();
    assert_eq!(error, expected);
}

#[test]
fn standard_token_counts_overflow_together() {
    let value = r#"{"predictions": [
        {"embeddings": {"values": [0.1], "statistics": {"token_count": 2147483648}}},
        {"embeddings": {"values": [0.2], "statistics": {"token_count": 2147483648}}}
    ]}"#;
    let error = transform_response(&json(value), "text-embedding-005").unwrap_err();
    assert_eq!(error, VertexTextEmbeddingError::TokenOverflow);
}

#[rstest]
#[case::standard(
    "text-embedding-005",
    r#"{"predictions": [{"embeddings": {"values": [0.1], "statistics": {"token_count": 1}}}]}"#
)]
#[case::bge("bge-small", r#"{"predictions": [[0.1]]}"#)]
#[case::tuned("42", r#"{"predictions": [[[0.1]]]}"#)]
fn answers_echo_the_requested_model(#[case] model: &str, #[case] value: &str) {
    assert_eq!(response(value, model).model, model);
}
