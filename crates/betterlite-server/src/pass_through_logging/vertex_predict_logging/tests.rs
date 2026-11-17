#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_vertex::gemini_embedding_records::error::GeminiEmbeddingError;
use betterlite_vertex::vertex_image_policy::error::PredictionError;
use betterlite_vertex::vertex_text_embeddings::error::VertexTextEmbeddingError;
use rstest::rstest;

use super::detect::{audio_prediction_count, is_multimodal_embedding};
use super::input::extract_input;
use super::{VertexPredictError, VertexPredictLogger};
use support::{
    details, embed_input, field, fixture, integer, json, kwargs, list, logged_call_type,
    logged_model, logged_provider, number_text, predict_input, priced, raw_fixture, result_json,
    text, unpriced,
};

mod support;

const PREDICT_ROUTE: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/project/locations/us-central1/publishers/google/models/text-embedding-004:predict";
const IMAGE_ROUTE: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/project/locations/us-central1/publishers/google/models/imagen-3.0-generate-001:predict";
const AUDIO_ROUTE: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/project/locations/us-central1/publishers/google/models/lyria-002:predict";
const EMBED_ROUTE: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/project/locations/us-central1/publishers/google/models/text-embedding-004:embedContent";
const BATCH_ROUTE: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/project/locations/us-central1/publishers/google/models/text-embedding-004:batchEmbedContents";
const GEMINI_ROUTE: &str =
    "https://generativelanguage.googleapis.com/v1beta/models/gemini-embedding-001:embedContent";

const AUDIO_BODY: &str =
    r#"{"predictions":[{"audioContent":"AAA"},{"bytesBase64Encoded":"BBB"},{"audioContent":""}]}"#;
const TEXT_BODY: &str = r#"{"predictions":[{"embeddings":{"values":[0.5,0.25],"statistics":{"truncated":false,"token_count":2}}},{"embeddings":{"values":[1.5],"statistics":{"truncated":false,"token_count":3}}}]}"#;

fn kwarg<'outcome>(
    outcome: &'outcome betterlite_logging::passthrough_stream_logging::types::HandlerOutcome,
    name: &str,
) -> &'outcome Json {
    outcome
        .kwargs
        .entries
        .get(name)
        .expect("the kwargs carry the key")
}

#[test]
fn audio_prices_lyria_predictions_by_their_count() {
    let fixture = fixture(AUDIO_ROUTE, AUDIO_BODY);
    let cost = priced();
    let logger = VertexPredictLogger::new(cost.clone());
    let outcome = logger
        .handle_predict_response(predict_input(&fixture))
        .expect("the audio reply is logged");
    let result = result_json(&outcome);
    assert_eq!(field(&result, "response"), &json(AUDIO_BODY));
    assert!(
        cost.calls().is_empty(),
        "the audio path prices the Lyria catalog itself"
    );
    assert_eq!(number_text(kwarg(&outcome, "response_cost")), "0.12");
    assert_eq!(text(kwarg(&outcome, "model")), "lyria-002");
    assert_eq!(text(kwarg(&outcome, "custom_llm_provider")), "vertex_ai");
    assert_eq!(logged_model(&fixture.logging_obj), "lyria-002");
    assert_eq!(
        logged_provider(&fixture.logging_obj).as_deref(),
        Some("vertex_ai")
    );
    assert_eq!(
        logged_call_type(&fixture.logging_obj),
        "pass_through_endpoint"
    );
    let all = details(&fixture.logging_obj);
    assert_eq!(
        number_text(all.get("response_cost").expect("the cost is written")),
        "0.12"
    );
    assert_eq!(
        text(all.get("model").expect("the model is written")),
        "lyria-002"
    );
}

#[test]
fn audio_zero_counts_fall_through_to_the_text_step() {
    let body = r#"{"predictions":[{"audioContent":"","embeddings":{"values":[0.5],"statistics":{"token_count":2}}}]}"#;
    let fixture = fixture(AUDIO_ROUTE, body);
    let cost = priced();
    let logger = VertexPredictLogger::new(cost.clone());
    let outcome = logger
        .handle_predict_response(predict_input(&fixture))
        .expect("the fall-through reply is logged");
    let result = result_json(&outcome);
    assert_eq!(text(field(&result, "model")), "lyria-002");
    assert_eq!(integer(field(field(&result, "usage"), "prompt_tokens")), 2);
    let calls = cost.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "lyria-002");
}

#[test]
fn audio_model_without_price_falls_through_to_text() {
    let body = r#"{"predictions":[{"audioContent":"AAA","embeddings":{"values":[0.5],"statistics":{"token_count":2}}}]}"#;
    let fixture = fixture(PREDICT_ROUTE, body);
    let cost = priced();
    let logger = VertexPredictLogger::new(cost.clone());
    let outcome = logger
        .handle_predict_response(predict_input(&fixture))
        .expect("the fall-through reply is logged");
    let result = result_json(&outcome);
    assert_eq!(text(field(&result, "model")), "text-embedding-004");
    assert_eq!(cost.calls().len(), 1);
    assert_eq!(number_text(kwarg(&outcome, "response_cost")), "0.0007");
}

#[rstest]
#[case("{}", 0)]
#[case(r#"{"predictions":{}}"#, 0)]
#[case(r#"{"predictions":[]}"#, 0)]
#[case(r#"{"predictions":[{"audioContent":"x"}]}"#, 1)]
#[case(r#"{"predictions":[{"audioContent":""}]}"#, 0)]
#[case(r#"{"predictions":[{"audioContent":null}]}"#, 0)]
#[case(r#"{"predictions":[{"audioContent":0}]}"#, 0)]
#[case(r#"{"predictions":[{"audioContent":[]}]}"#, 0)]
#[case(r#"{"predictions":[{"audioContent":{}}]}"#, 0)]
#[case(r#"{"predictions":[{"audioContent":[0]}]}"#, 1)]
#[case(r#"{"predictions":[{"bytesBase64Encoded":1}]}"#, 1)]
#[case(r#"{"predictions":[{"bytesBase64Encoded":false}]}"#, 0)]
#[case(r#"{"predictions":[1,"x",null]}"#, 0)]
#[case(
    r#"{"predictions":[{"audioContent":"a"},{"bytesBase64Encoded":"b"}]}"#,
    2
)]
fn audio_count_uses_python_truthiness(#[case] body: &str, #[case] expected: usize) {
    let Json::Object(fields) = json(body) else {
        panic!("the case body is an object");
    };
    assert_eq!(audio_prediction_count(&fields), expected, "{body}");
}

#[test]
fn predict_text_embedding_builds_the_response_and_prices_it() {
    let fixture = fixture(PREDICT_ROUTE, TEXT_BODY);
    let cost = priced();
    let logger = VertexPredictLogger::new(cost.clone());
    let outcome = logger
        .handle_predict_response(predict_input(&fixture))
        .expect("the embedding reply is logged");
    let result = result_json(&outcome);
    assert_eq!(text(field(&result, "object")), "list");
    assert_eq!(text(field(&result, "model")), "text-embedding-004");
    let data = list(field(&result, "data"));
    assert_eq!(data.len(), 2);
    assert_eq!(text(field(&data[0], "object")), "embedding");
    assert_eq!(text(field(&data[1], "object")), "embedding");
    assert_eq!(integer(field(&data[1], "index")), 1);
    assert_eq!(integer(field(&data[0], "index")), 0);
}

#[test]
fn predict_text_embedding_sums_the_token_counts() {
    let body = TEXT_BODY;
    let fixture = fixture(PREDICT_ROUTE, body);
    let cost = priced();
    let logger = VertexPredictLogger::new(cost.clone());
    let outcome = logger
        .handle_predict_response(predict_input(&fixture))
        .expect("the embedding reply is logged");
    let result = result_json(&outcome);
    let usage = field(&result, "usage");
    assert_eq!(integer(field(usage, "prompt_tokens")), 5);
    assert_eq!(integer(field(usage, "completion_tokens")), 0);
    assert_eq!(integer(field(usage, "total_tokens")), 5);
    let calls = cost.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "text-embedding-004");
    assert_eq!(calls[0].provider, "vertex_ai");
    assert_eq!(calls[0].location.as_deref(), Some("us-central1"));
    assert_eq!(
        calls[0].response_model.as_deref(),
        Some("text-embedding-004")
    );
    assert_eq!(calls[0].usage, Some((5, 0, 5)));
    assert_eq!(number_text(kwarg(&outcome, "response_cost")), "0.0007");
    assert_eq!(logged_model(&fixture.logging_obj), "text-embedding-004");
    assert_eq!(
        logged_provider(&fixture.logging_obj).as_deref(),
        Some("vertex_ai")
    );
    assert_eq!(
        logged_call_type(&fixture.logging_obj),
        "pass_through_endpoint"
    );
    let all = details(&fixture.logging_obj);
    assert_eq!(
        text(all.get("model").expect("the model is written")),
        "text-embedding-004"
    );
    assert_eq!(
        text(
            all.get("custom_llm_provider")
                .expect("the provider is written")
        ),
        "vertex_ai"
    );
    assert_eq!(
        number_text(all.get("response_cost").expect("the cost is written")),
        "0.0007"
    );
}

#[test]
fn predict_image_generation_sets_the_call_type() {
    let body = r#"{"predictions":[{"bytesBase64Encoded":"AAA"},{"bytesBase64Encoded":"BBB"}]}"#;
    let fixture = fixture(IMAGE_ROUTE, body);
    let cost = priced();
    let logger = VertexPredictLogger::new(cost.clone());
    let outcome = logger
        .handle_predict_response(predict_input(&fixture))
        .expect("the image reply is logged");
    let result = result_json(&outcome);
    let data = list(field(&result, "data"));
    assert_eq!(data.len(), 2);
    assert_eq!(text(field(&data[0], "b64_json")), "AAA");
    assert_eq!(text(field(&data[1], "b64_json")), "BBB");
    assert_eq!(
        logged_call_type(&fixture.logging_obj),
        "passthrough-image-generation"
    );
    assert_eq!(
        logged_model(&fixture.logging_obj),
        "imagen-3.0-generate-001"
    );
    let calls = cost.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "imagen-3.0-generate-001");
    assert_eq!(calls[0].provider, "vertex_ai");
    assert_eq!(calls[0].location.as_deref(), Some("us-central1"));
    assert_eq!(calls[0].response_model, None);
    assert_eq!(calls[0].usage, Some((0, 0, 0)));
    assert_eq!(number_text(kwarg(&outcome, "response_cost")), "0.0007");
    assert_eq!(text(kwarg(&outcome, "custom_llm_provider")), "vertex_ai");
}

#[test]
fn predict_cost_failure_is_an_error() {
    let fixture = fixture(PREDICT_ROUTE, TEXT_BODY);
    let logger = VertexPredictLogger::new(unpriced());
    let Err(error) = logger.handle_predict_response(predict_input(&fixture)) else {
        panic!("an unpriced model is an error");
    };
    assert!(
        matches!(&error, VertexPredictError::Cost { model } if model == "text-embedding-004"),
        "{error}"
    );
    assert!(details(&fixture.logging_obj).is_empty());
    assert_eq!(logged_model(&fixture.logging_obj), "");
    assert_eq!(logged_provider(&fixture.logging_obj), None);
    assert_eq!(
        logged_call_type(&fixture.logging_obj),
        "pass_through_endpoint"
    );
}

#[test]
fn predict_invalid_json_body_is_a_decode_error() {
    let fixture = raw_fixture(PREDICT_ROUTE, b"{oops");
    let logger = VertexPredictLogger::new(priced());
    let Err(error) = logger.handle_predict_response(predict_input(&fixture)) else {
        panic!("an invalid body is an error");
    };
    assert!(
        matches!(&error, VertexPredictError::Decode { .. }),
        "{error}"
    );
}

#[test]
fn predict_non_object_body_is_a_shape_error() {
    let fixture = fixture(PREDICT_ROUTE, "[]");
    let logger = VertexPredictLogger::new(priced());
    let Err(error) = logger.handle_predict_response(predict_input(&fixture)) else {
        panic!("a non-object body is an error");
    };
    assert!(
        matches!(&error, VertexPredictError::BodyShape { found } if *found == "a list"),
        "{error}"
    );
}

#[test]
fn predict_empty_predictions_take_the_image_error() {
    let fixture = fixture(PREDICT_ROUTE, r#"{"predictions":[]}"#);
    let logger = VertexPredictLogger::new(priced());
    let Err(error) = logger.handle_predict_response(predict_input(&fixture)) else {
        panic!("an empty predictions list is an error");
    };
    assert!(
        matches!(
            &error,
            VertexPredictError::Prediction(PredictionError::EmptyPredictions)
        ),
        "{error}"
    );
}

#[test]
fn predict_missing_predictions_is_a_text_embedding_error() {
    let fixture = fixture(PREDICT_ROUTE, r#"{"answers":[1]}"#);
    let logger = VertexPredictLogger::new(priced());
    let Err(error) = logger.handle_predict_response(predict_input(&fixture)) else {
        panic!("a body without predictions is an error");
    };
    assert!(
        matches!(
            &error,
            VertexPredictError::TextEmbedding(VertexTextEmbeddingError::MissingPredictions)
        ),
        "{error}"
    );
}

#[test]
fn predict_kwargs_keep_the_caller_entries() {
    let fixture = fixture(PREDICT_ROUTE, TEXT_BODY);
    let logger = VertexPredictLogger::new(priced());
    let mut input = predict_input(&fixture);
    input.kwargs = kwargs(&[("user", Json::Text("abc".to_owned()))]);
    let outcome = logger
        .handle_predict_response(input)
        .expect("the embedding reply is logged");
    assert_eq!(outcome.kwargs.entries.len(), 4);
    assert_eq!(text(kwarg(&outcome, "user")), "abc");
    assert_eq!(text(kwarg(&outcome, "model")), "text-embedding-004");
}

#[rstest]
#[case("{}", false)]
#[case(r#"{"predictions":[]}"#, false)]
#[case(r#"{"predictions":"x"}"#, false)]
#[case(r#"{"predictions":[{"embeddings":{}}]}"#, false)]
#[case(r#"{"predictions":[{"textEmbedding":[0.5]}]}"#, true)]
#[case(r#"{"predictions":[{"textEmbedding":null}]}"#, true)]
#[case(r#"{"predictions":[{"imageEmbedding":[]}]}"#, true)]
#[case(r#"{"predictions":[{"videoEmbeddings":[]}]}"#, true)]
#[case(r#"{"predictions":[1,"x",{"textEmbedding":1}]}"#, true)]
#[case(r#"{"predictions":[{"other":1},{"imageEmbedding":null}]}"#, true)]
fn multimodal_detection_needs_the_key(#[case] body: &str, #[case] expected: bool) {
    let Json::Object(fields) = json(body) else {
        panic!("the case body is an object");
    };
    assert_eq!(is_multimodal_embedding(&fields), expected, "{body}");
}

#[test]
fn multimodal_body_fails_in_the_shared_step() {
    let fixture = fixture(
        "https://us-central1-aiplatform.googleapis.com/v1/projects/project/locations/us-central1/publishers/google/models/multimodalembedding@001:predict",
        r#"{"predictions":[{"textEmbedding":[0.5]}]}"#,
    );
    let cost = priced();
    let logger = VertexPredictLogger::new(cost.clone());
    let Err(error) = logger.handle_predict_response(predict_input(&fixture)) else {
        panic!("the multimodal step rejects the empty request");
    };
    assert!(
        matches!(&error, VertexPredictError::Multimodal(_)),
        "{error}"
    );
    assert!(cost.calls().is_empty());
    assert!(details(&fixture.logging_obj).is_empty());
}

#[test]
fn multimodal_key_in_a_later_entry_is_detected() {
    let fixture = fixture(
        "https://us-central1-aiplatform.googleapis.com/v1/projects/project/locations/us-central1/publishers/google/models/multimodalembedding@001:predict",
        r#"{"predictions":[{},{"other":1},{"videoEmbeddings":[]}]}"#,
    );
    let logger = VertexPredictLogger::new(priced());
    let Err(error) = logger.handle_predict_response(predict_input(&fixture)) else {
        panic!("the multimodal step rejects the empty request");
    };
    assert!(
        matches!(&error, VertexPredictError::Multimodal(_)),
        "{error}"
    );
}

#[test]
fn embed_single_joins_the_parts_and_prices_it() {
    let fixture = fixture(EMBED_ROUTE, r#"{"embedding":{"values":[0.5,0.25]}}"#);
    let request = json(
        r#"{"content":{"parts":[{"text":"hello"},{"inlineData":{"data":"x"}},{"text":"world"}]}}"#,
    );
    let cost = priced();
    let logger = VertexPredictLogger::new(cost.clone());
    let outcome = logger
        .handle_embed_content_response(embed_input(&fixture, Some(&request)))
        .expect("the embedContent reply is logged");
    let result = result_json(&outcome);
    assert_eq!(text(field(&result, "object")), "list");
    assert_eq!(text(field(&result, "model")), "text-embedding-004");
    assert_eq!(list(field(&result, "data")).len(), 1);
    let usage = field(&result, "usage");
    assert_eq!(integer(field(usage, "prompt_tokens")), 12);
    assert_eq!(integer(field(usage, "total_tokens")), 12);
    assert_eq!(field(usage, "prompt_tokens_details"), &Json::Null);
    let calls = cost.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "text-embedding-004");
    assert_eq!(calls[0].provider, "vertex_ai");
    assert_eq!(calls[0].location.as_deref(), Some("us-central1"));
    assert_eq!(calls[0].usage, Some((12, 0, 12)));
    assert_eq!(number_text(kwarg(&outcome, "response_cost")), "0.0007");
    assert_eq!(text(kwarg(&outcome, "custom_llm_provider")), "vertex_ai");
    let all = details(&fixture.logging_obj);
    assert_eq!(
        text(all.get("model").expect("the model is written")),
        "text-embedding-004"
    );
    assert_eq!(
        logged_call_type(&fixture.logging_obj),
        "pass_through_endpoint"
    );
}

#[test]
fn embed_batch_joins_every_request_part() {
    let fixture = fixture(
        BATCH_ROUTE,
        r#"{"embeddings":[{"values":[0.5]},{"values":[1.5,2.5]}]}"#,
    );
    let request = json(
        r#"{"requests":[{"content":{"parts":[{"text":"ab"}]}},{"content":{"parts":[{"text":"c"},{"text":"d"}]}}]}"#,
    );
    let cost = priced();
    let logger = VertexPredictLogger::new(cost.clone());
    let outcome = logger
        .handle_embed_content_response(embed_input(&fixture, Some(&request)))
        .expect("the batchEmbedContents reply is logged");
    let result = result_json(&outcome);
    assert_eq!(list(field(&result, "data")).len(), 2);
    let usage = field(&result, "usage");
    assert_eq!(integer(field(usage, "prompt_tokens")), 6);
    assert_eq!(integer(field(usage, "total_tokens")), 6);
    assert_eq!(cost.calls()[0].usage, Some((6, 0, 6)));
}

#[test]
fn embed_gemini_host_prices_as_gemini() {
    let fixture = fixture(GEMINI_ROUTE, r#"{"embedding":{"values":[0.5]}}"#);
    let cost = priced();
    let logger = VertexPredictLogger::new(cost.clone());
    let outcome = logger
        .handle_embed_content_response(embed_input(&fixture, None))
        .expect("the gemini reply is logged");
    let result = result_json(&outcome);
    assert_eq!(text(field(&result, "model")), "gemini-embedding-001");
    assert_eq!(integer(field(field(&result, "usage"), "prompt_tokens")), 0);
    let calls = cost.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "gemini-embedding-001");
    assert_eq!(calls[0].provider, "gemini");
    assert_eq!(calls[0].location, None);
    assert_eq!(
        calls[0].response_model.as_deref(),
        Some("gemini-embedding-001")
    );
    assert_eq!(text(kwarg(&outcome, "custom_llm_provider")), "gemini");
    assert_eq!(
        logged_provider(&fixture.logging_obj).as_deref(),
        Some("gemini")
    );
    let all = details(&fixture.logging_obj);
    assert_eq!(
        text(
            all.get("custom_llm_provider")
                .expect("the provider is written")
        ),
        "gemini"
    );
}

#[test]
fn embed_absent_request_body_counts_nothing() {
    let fixture = fixture(EMBED_ROUTE, r#"{"embedding":{"values":[0.5]}}"#);
    let cost = priced();
    let logger = VertexPredictLogger::new(cost.clone());
    let outcome = logger
        .handle_embed_content_response(embed_input(&fixture, None))
        .expect("the reply without a request body is logged");
    let result = result_json(&outcome);
    let usage = field(&result, "usage");
    assert_eq!(integer(field(usage, "prompt_tokens")), 0);
    assert_eq!(integer(field(usage, "total_tokens")), 0);
    assert_eq!(cost.calls()[0].usage, Some((0, 0, 0)));
}

#[test]
fn embed_non_object_request_body_counts_nothing() {
    let fixture = fixture(EMBED_ROUTE, r#"{"embedding":{"values":[0.5]}}"#);
    let request = json("[1]");
    let cost = priced();
    let logger = VertexPredictLogger::new(cost.clone());
    let outcome = logger
        .handle_embed_content_response(embed_input(&fixture, Some(&request)))
        .expect("a non-object request body is an empty input");
    let result = result_json(&outcome);
    assert_eq!(integer(field(field(&result, "usage"), "prompt_tokens")), 0);
}

#[test]
fn embed_usage_metadata_details_are_kept() {
    let fixture = fixture(
        EMBED_ROUTE,
        r#"{"embedding":{"values":[0.5]},"usageMetadata":{"promptTokenCount":3,"totalTokenCount":5,"promptTokensDetails":[{"modality":"TEXT","tokenCount":2},{"modality":"IMAGE","tokenCount":1}]}}"#,
    );
    let request = json(r#"{"content":{"parts":[{"text":"hi"}]}}"#);
    let cost = priced();
    let logger = VertexPredictLogger::new(cost.clone());
    let outcome = logger
        .handle_embed_content_response(embed_input(&fixture, Some(&request)))
        .expect("the reply with usage metadata is logged");
    let result = result_json(&outcome);
    let usage = field(&result, "usage");
    assert_eq!(integer(field(usage, "prompt_tokens")), 3);
    assert_eq!(integer(field(usage, "total_tokens")), 5);
    let rendered = field(usage, "prompt_tokens_details");
    assert_eq!(integer(field(rendered, "text_tokens")), 2);
    assert_eq!(integer(field(rendered, "audio_tokens")), 0);
    assert_eq!(integer(field(rendered, "image_tokens")), 1);
    assert_eq!(integer(field(rendered, "video_tokens")), 0);
    assert_eq!(cost.calls()[0].usage, Some((3, 0, 5)));
}

#[test]
fn embed_cost_failure_is_an_error() {
    let fixture = fixture(EMBED_ROUTE, r#"{"embedding":{"values":[0.5]}}"#);
    let logger = VertexPredictLogger::new(unpriced());
    let Err(error) = logger.handle_embed_content_response(embed_input(&fixture, None)) else {
        panic!("an unpriced model is an error");
    };
    assert!(
        matches!(&error, VertexPredictError::Cost { model } if model == "text-embedding-004"),
        "{error}"
    );
    assert!(details(&fixture.logging_obj).is_empty());
}

#[test]
fn embed_body_without_embedding_is_an_error() {
    let fixture = fixture(EMBED_ROUTE, r#"{"values":[0.5]}"#);
    let logger = VertexPredictLogger::new(priced());
    let Err(error) = logger.handle_embed_content_response(embed_input(&fixture, None)) else {
        panic!("a reply without an embedding is an error");
    };
    assert!(
        matches!(
            &error,
            VertexPredictError::GeminiEmbedding(GeminiEmbeddingError::Reply { .. })
        ),
        "{error}"
    );
}

#[test]
fn embed_batch_body_without_embeddings_is_an_error() {
    let fixture = fixture(BATCH_ROUTE, r#"{"values":[0.5]}"#);
    let logger = VertexPredictLogger::new(priced());
    let Err(error) = logger.handle_embed_content_response(embed_input(&fixture, None)) else {
        panic!("a reply without embeddings is an error");
    };
    assert!(
        matches!(
            &error,
            VertexPredictError::GeminiEmbedding(GeminiEmbeddingError::Reply { .. })
        ),
        "{error}"
    );
}

#[test]
fn embed_invalid_json_body_is_a_decode_error() {
    let fixture = raw_fixture(EMBED_ROUTE, b"{oops");
    let logger = VertexPredictLogger::new(priced());
    let Err(error) = logger.handle_embed_content_response(embed_input(&fixture, None)) else {
        panic!("an invalid body is an error");
    };
    assert!(
        matches!(&error, VertexPredictError::Decode { .. }),
        "{error}"
    );
}

#[rstest]
#[case(None, false, "")]
#[case(
    Some(r#"{"content":{"parts":[{"text":"a"},{"text":"b"}]}}"#),
    false,
    "a b"
)]
#[case(
    Some(r#"{"content":{"parts":[{"text":5},{"text":"b"}]}}"#),
    false,
    " b"
)]
#[case(Some(r#"{"content":{"parts":[{"inlineData":{}}]}}"#), false, "")]
#[case(Some(r#"{"content":{"parts":[{"text":"x"}]}}"#), true, "")]
#[case(Some(r#"{"requests":[{"content":{"parts":[{"text":"a"}]}},{"content":{"parts":[{"text":"b"},{"text":"c"}]}}]}"#), true, "a b c")]
#[case(Some("[1]"), false, "")]
fn embed_inputs_join_the_part_text(
    #[case] request: Option<&str>,
    #[case] batch: bool,
    #[case] expected: &str,
) {
    let parsed = request.map(json);
    assert_eq!(extract_input(parsed.as_ref(), batch), expected);
}
