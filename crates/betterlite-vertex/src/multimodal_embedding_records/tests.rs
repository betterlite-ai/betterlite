#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::str::from_utf8;

use betterlite_models::decisions::Extra;
use betterlite_models::decisions::Json;
use betterlite_openai::embedding::EmbeddingVector;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::SecretString;

use super::error::INTERNAL_SERVER_ERROR;
use super::error::MultimodalEmbeddingError;
use super::request::MEDIA_TEXT_LIMIT;
use super::request::MediaKind;
use super::request::SUPPORTED_OPENAI_PARAMS;
use super::request::classify_embedding_media;
use super::request::element;
use super::request::map_openai_params;
use super::request::transform_request;
use super::request::validate_environment;
use super::response::MultimodalEmbeddingAnswer;
use super::response::RESPONSE_BODY_LIMIT;
use super::response::decode_multimodal_embedding;
use super::response::transform_response;
use super::usage::calculate_usage;

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn extra(source: &str) -> Extra {
    serde_json::from_str(source).unwrap()
}

fn vector(source: &str) -> EmbeddingVector {
    serde_json::from_str(source).unwrap()
}

fn items(source: &str) -> Vec<Json> {
    serde_json::from_str(source).unwrap()
}

fn request(input: &str, optional: &str) -> Json {
    transform_request(&json(input), &extra(optional)).unwrap()
}

fn answer(response: &str, request: &str) -> MultimodalEmbeddingAnswer {
    transform_response(
        200,
        response.as_bytes(),
        &json(request),
        "multimodalembedding@001",
    )
    .unwrap()
}

#[test]
fn supported_openai_params_is_dimensions_only() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["dimensions"]);
}

#[test]
fn dimensions_map_to_output_dimensionality() {
    let mapped = map_openai_params(
        &extra(r#"{"dimensions": 8, "user": "u"}"#),
        extra(r#"{"dimensions": 9}"#),
    );
    assert_eq!(
        mapped,
        extra(r#"{"dimensions": 9, "outputDimensionality": 8}"#)
    );
}

#[test]
fn headers_force_the_content_type_and_bearer_after_the_caller() {
    let mut caller = BTreeMap::new();
    caller.insert(String::from("Content-Type"), String::from("text/plain"));
    caller.insert(String::from("Authorization"), String::from("Bearer caller"));
    caller.insert(String::from("content-type"), String::from("text/plain"));
    caller.insert(String::from("X-Trace"), String::from("one"));
    let headers = validate_environment(caller, &SecretString::from("vertex-key"));
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json; charset=utf-8")
    );
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer vertex-key")
    );
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("text/plain")
    );
    assert_eq!(headers.get("X-Trace").map(String::as_str), Some("one"));
}

#[test]
fn text_then_image_merges_the_following_media() {
    let body = request(
        r#"["a red bike", "gs://bucket/bike.png", "a blue bike", "data:image/png;base64,YWJj"]"#,
        "{}",
    );
    assert_eq!(
        body,
        json(
            r#"{"instances": [
                {"text": "a red bike", "image": {"gcsUri": "gs://bucket/bike.png"}},
                {"text": "a blue bike", "image": {"bytesBase64Encoded": "YWJj"}}
            ]}"#
        )
    );
}

#[test]
fn image_then_text_keeps_the_leading_media_standalone() {
    let body = request(
        r#"["gs://bucket/bike.png", "a red bike", "plain trailing text"]"#,
        "{}",
    );
    assert_eq!(
        body,
        json(
            r#"{"instances": [
                {"image": {"gcsUri": "gs://bucket/bike.png"}},
                {"text": "a red bike"},
                {"text": "plain trailing text"}
            ]}"#
        )
    );
}

#[test]
fn substring_video_only_turns_gcs_inputs_into_video() {
    let body = request(
        r#"["gs://bucket/clip.mp4", "gs://bucket/photo.png", "https://cdn/clip.mp4", "data:video/mp4;base64,YWJj", "tail"]"#,
        "{}",
    );
    assert_eq!(
        body,
        json(
            r#"{"instances": [
                {"video": {"gcsUri": "gs://bucket/clip.mp4"}},
                {"image": {"gcsUri": "gs://bucket/photo.png"}},
                {"text": "https://cdn/clip.mp4", "image": {"bytesBase64Encoded": "YWJj"}},
                {"text": "tail"}
            ]}"#
        )
    );
}

#[test]
fn mapping_elements_pass_through_as_native_instances() {
    let body = request(
        r#"[{"text": "native", "image": {"gcsUri": "gs://bucket/i.png"}}, "caption"]"#,
        "{}",
    );
    assert_eq!(
        body,
        json(
            r#"{"instances": [
                {"text": "native", "image": {"gcsUri": "gs://bucket/i.png"}},
                {"text": "caption"}
            ]}"#
        )
    );
}

#[test]
fn an_empty_element_does_not_merge_with_the_text_before_it() {
    let body = request(r#"["a", ""]"#, "{}");
    assert_eq!(
        body,
        json(r#"{"instances": [{"text": "a"}, {"text": ""}]}"#)
    );
}

#[test]
fn explicit_instances_win_over_the_input() {
    let body = request(
        r#"["ignored", "gs://bucket/ignored.png"]"#,
        r#"{"instances": [{"text": "native"}], "outputDimensionality": 4}"#,
    );
    assert_eq!(
        body,
        json(r#"{"instances": [{"text": "native"}], "parameters": {"dimension": 4}}"#)
    );
}

#[test]
fn a_scalar_string_replaces_the_optional_instance_fields() {
    let body = request(r#""hello""#, r#"{"task_type": "RETRIEVAL_QUERY"}"#);
    assert_eq!(body, json(r#"{"instances": [{"text": "hello"}]}"#));
    let media = request(r#""gs://bucket/clip.mp4""#, "{}");
    assert_eq!(
        media,
        json(r#"{"instances": [{"video": {"gcsUri": "gs://bucket/clip.mp4"}}]}"#)
    );
}

#[test]
fn an_unsupported_element_is_a_client_error() {
    let element_error = transform_request(&json("[1]"), &Extra::new()).unwrap_err();
    assert_eq!(element_error.status_code(), 400);
    let input_error = transform_request(&json("42"), &Extra::new()).unwrap_err();
    assert!(matches!(
        input_error,
        MultimodalEmbeddingError::UnsupportedInput
    ));
}

#[rstest]
#[case::gcs_image("gs://bucket/a.png", Some(MediaKind::GcsImage))]
#[case::gcs_video("gs://bucket/a.mp4", Some(MediaKind::GcsVideo))]
#[case::gcs_substring("prefix gs://bucket/a", Some(MediaKind::GcsImage))]
#[case::mp4_outside_gcs("https://cdn/a.mp4", None)]
#[case::base64("data:image/png;base64,YWJj", Some(MediaKind::Base64Image))]
#[case::comma_segment("data:image/png;base64,YWJj,extra", Some(MediaKind::Base64Image))]
#[case::comma_only("data:,", Some(MediaKind::Base64Image))]
#[case::no_comma("data:image/png;base64", None)]
#[case::bad_padding("data:image/png;base64,YWJ", None)]
#[case::not_data_prefix("YWJj", None)]
#[case::plain("Dog", None)]
#[case::empty("", None)]
fn media_classification_follows_litellm(#[case] input: &str, #[case] expected: Option<MediaKind>) {
    assert_eq!(classify_embedding_media(input), expected);
}

#[test]
fn base64_images_keep_only_the_second_comma_segment() {
    let comma = "data:image/png;base64,YWJj,extra";
    assert_eq!(
        element(None, comma, classify_embedding_media(comma)),
        json(r#"{"image": {"bytesBase64Encoded": "YWJj"}}"#)
    );
    let payload = "data:,ABCD";
    assert_eq!(
        element(None, payload, classify_embedding_media(payload)),
        json(r#"{"image": {"bytesBase64Encoded": "ABCD"}}"#)
    );
}

#[test]
fn media_classification_stops_at_the_text_limit() {
    let header = "data:xx,";
    let at_limit = format!(
        "{header}{}",
        "A".repeat(MEDIA_TEXT_LIMIT.saturating_sub(header.len()))
    );
    assert_eq!(at_limit.len(), MEDIA_TEXT_LIMIT);
    assert_eq!(
        classify_embedding_media(&at_limit),
        Some(MediaKind::Base64Image)
    );
    let over = format!(
        "{header}{}",
        "A".repeat(
            MEDIA_TEXT_LIMIT
                .saturating_sub(header.len())
                .saturating_add(4)
        )
    );
    assert_eq!(over.len(), MEDIA_TEXT_LIMIT.saturating_add(4));
    assert_eq!(classify_embedding_media(&over), None);
}

#[test]
fn text_vector_precedence_skips_image_and_video_vectors() {
    let response = answer(
        r#"{"predictions": [{"textEmbedding": [0.1, 0.2], "imageEmbedding": [0.3], "videoEmbeddings": [{"startOffsetSec": 0, "endOffsetSec": 2, "embedding": [0.4]}]}]}"#,
        r#"{"instances": [{"text": "a"}, {"image": {"gcsUri": "gs://bucket/i.png"}}]}"#,
    );
    assert_eq!(response.response.object, "list");
    assert_eq!(response.response.model, "multimodalembedding@001");
    assert_eq!(response.response.data.len(), 1);
    assert_eq!(response.response.data[0].object, "embedding");
    assert_eq!(response.response.data[0].index, 0);
    assert_eq!(response.response.data[0].embedding, vector("[0.1, 0.2]"));
    assert_eq!(response.response.usage.prompt_tokens, 0);
    assert_eq!(response.response.usage.completion_tokens, 0);
    assert_eq!(response.response.usage.total_tokens, 0);
    assert_eq!(response.details.character_count, Some(1));
    assert_eq!(response.details.image_count, 1);
    assert_eq!(response.details.video_length_seconds, Decimal::from(2_u32));
}

#[test]
fn image_vector_precedence_skips_video_vectors() {
    let response = answer(
        r#"{"predictions": [{"imageEmbedding": [0.5], "videoEmbeddings": [{"startOffsetSec": 0, "endOffsetSec": 2, "embedding": [0.6]}]}]}"#,
        r#"{"instances": []}"#,
    );
    assert_eq!(response.response.data.len(), 1);
    assert_eq!(response.response.data[0].index, 0);
    assert_eq!(response.response.data[0].embedding, vector("[0.5]"));
    assert_eq!(response.details.video_length_seconds, Decimal::from(2_u32));
}

#[test]
fn repeated_video_index_keeps_the_prediction_index() {
    let response = answer(
        r#"{"predictions": [
            {"videoEmbeddings": [
                {"startOffsetSec": 0, "endOffsetSec": 1, "embedding": [0.1]},
                {"startOffsetSec": 1, "endOffsetSec": 3, "embedding": [0.2]}
            ]},
            {"imageEmbedding": [0.3]}
        ]}"#,
        r#"{"instances": []}"#,
    );
    assert_eq!(response.response.data.len(), 3);
    assert_eq!(response.response.data[0].index, 0);
    assert_eq!(response.response.data[1].index, 0);
    assert_eq!(response.response.data[2].index, 1);
    assert_eq!(response.response.data[0].embedding, vector("[0.1]"));
    assert_eq!(response.response.data[2].embedding, vector("[0.3]"));
    assert_eq!(response.details.character_count, None);
    assert_eq!(response.details.image_count, 0);
    assert_eq!(response.details.video_length_seconds, Decimal::from(3_u32));
}

#[test]
fn an_empty_prediction_produces_no_record() {
    let response = answer(
        r#"{"predictions": [{}, {"textEmbedding": [0.5]}]}"#,
        r#"{"instances": []}"#,
    );
    assert_eq!(response.response.data.len(), 1);
    assert_eq!(response.response.data[0].index, 1);
}

#[test]
fn whitespace_character_usage_counts_only_non_whitespace_characters() {
    let details = calculate_usage(
        &json(r#"{"instances": [{"text": " a\tb\nc\u001c "}, {"text": ""}, {"image": {}}]}"#),
        &[],
    )
    .unwrap();
    assert_eq!(details.character_count, Some(3));
    assert_eq!(details.image_count, 0);
    assert_eq!(details.video_length_seconds, Decimal::ZERO);
}

#[test]
fn empty_text_leaves_character_count_absent() {
    let details = calculate_usage(&json(r#"{"instances": [{"text": ""}]}"#), &[]).unwrap();
    assert_eq!(details.character_count, None);
}

#[test]
fn concatenated_text_counts_one_character_count() {
    let details = calculate_usage(
        &json(r#"{"instances": [{"text": "ab"}, {"text": "c d"}]}"#),
        &[],
    )
    .unwrap();
    assert_eq!(details.character_count, Some(4));
}

#[test]
fn video_usage_skips_absent_segments() {
    let details = calculate_usage(
        &json(r#"{"instances": []}"#),
        &items(r#"[{"textEmbedding": [0.1]}, {"videoEmbeddings": null}]"#),
    )
    .unwrap();
    assert_eq!(details.video_length_seconds, Decimal::ZERO);
}

#[test]
fn video_usage_rejects_a_segment_without_offsets() {
    let error = calculate_usage(
        &json(r#"{"instances": []}"#),
        &items(r#"[{"videoEmbeddings": [{"embedding": [0.1]}]}]"#),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        MultimodalEmbeddingError::Field("startOffsetSec", "number", 0)
    ));
}

#[test]
fn video_usage_accepts_exponent_form_offsets() {
    let details = calculate_usage(
        &json(r#"{"instances": []}"#),
        &items(
            r#"[{"videoEmbeddings": [{"startOffsetSec": 1e-7, "endOffsetSec": 2e-7, "embedding": [0.1]}]}]"#,
        ),
    )
    .unwrap();
    assert_eq!(
        details.video_length_seconds,
        Decimal::from_scientific("1e-7").unwrap()
    );
}

#[test]
fn a_reply_without_predictions_is_an_internal_server_error() {
    let error = transform_response(200, br#"{"data": []}"#, &json("{}"), "m").unwrap_err();
    assert!(matches!(
        error,
        MultimodalEmbeddingError::MissingPredictions
    ));
    assert_eq!(error.status_code(), 500);
    assert_eq!(INTERNAL_SERVER_ERROR, "internal_server_error");
}

#[test]
fn a_reply_that_is_not_200_is_refused() {
    let error = transform_response(500, br#"{"predictions": []}"#, &json("{}"), "m").unwrap_err();
    assert!(matches!(
        error,
        MultimodalEmbeddingError::Status { status: 500 }
    ));
    assert_eq!(error.status_code(), 500);
}

#[test]
fn the_response_body_limit_is_exact() {
    let start = br#"{"predictions":[],"pad":""#;
    let end = br#""}"#;
    let mut at_limit = start.to_vec();
    at_limit.resize(RESPONSE_BODY_LIMIT.saturating_sub(end.len()), b'a');
    at_limit.extend_from_slice(end);
    assert_eq!(at_limit.len(), RESPONSE_BODY_LIMIT);
    decode_multimodal_embedding(&at_limit).unwrap();
    at_limit.push(b' ');
    let error = decode_multimodal_embedding(&at_limit).unwrap_err();
    assert!(matches!(
        error,
        MultimodalEmbeddingError::ResponseTooLarge { size, limit }
            if size == RESPONSE_BODY_LIMIT.saturating_add(1) && limit == RESPONSE_BODY_LIMIT
    ));
}

#[test]
fn a_malformed_reply_is_a_decode_error() {
    let error = decode_multimodal_embedding(b"{").unwrap_err();
    assert!(matches!(error, MultimodalEmbeddingError::Decode { .. }));
    assert_eq!(error.status_code(), 500);
}

#[rstest]
fn replays_the_multimodal_embedding_corpus(
    #[files("fuzz/corpus/multimodal_embedding/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    if let Ok(text) = from_utf8(&bytes) {
        let _classified = classify_embedding_media(text);
    }
    drop(decode_multimodal_embedding(&bytes));
}
