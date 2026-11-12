use rstest::rstest;

use super::support::CharacterCount;
use super::support::FailingCount;
use super::support::input;
use super::support::json;
use super::support::resolved_files;
use crate::gemini_embedding_records::error::GeminiEmbeddingError;
use crate::gemini_embedding_records::usage::EmbeddingUsage;
use crate::gemini_embedding_records::usage::ModalityTokens;
use crate::gemini_embedding_records::usage::batch_usage;
use crate::gemini_embedding_records::usage::embed_content_usage;

const MODEL: &str = "gemini-embedding-001";

fn characters(text: &str) -> u32 {
    u32::try_from(text.chars().count()).unwrap()
}

fn count_usage(source: &str) -> EmbeddingUsage {
    batch_usage(&input(source), MODEL, &CharacterCount).unwrap()
}

#[rstest]
#[case::text(r#""hello""#, "hello")]
#[case::texts(r#"["ab","c"]"#, "abc")]
#[case::nested(r#"[["ab","c"],"d"]"#, "abcd")]
fn batch_counts_the_concatenated_input(#[case] source: &str, #[case] counted: &str) {
    assert_eq!(count_usage(source).prompt_tokens, characters(counted));
    assert_eq!(count_usage(source).total_tokens, characters(counted));
    assert_eq!(count_usage(source).prompt_tokens_details, None);
}

#[rstest]
#[case::media_and_text(r#"["data:image/png;base64,AAAA","ab"]"#, "ab")]
#[case::nested_media_and_text(r#"[["gs://bucket/x.png","ab"]]"#, "ab")]
#[case::http_url_is_text(
    r#"["https://example.com/a.png","data:image/png;base64,AA"]"#,
    "https://example.com/a.png"
)]
#[case::data_url_without_marker_is_text(r#"["data:image/png,AA"]"#, "data:image/png,AA")]
fn batch_counts_text_elements_only(#[case] source: &str, #[case] counted: &str) {
    assert_eq!(count_usage(source).prompt_tokens, characters(counted));
}

#[rstest]
#[case::all_media(r#"["data:image/png;base64,AAAA","gs://bucket/x.png","files/f"]"#)]
#[case::empty_group("[[]]")]
#[case::empty_list("[]")]
fn batch_counts_nothing_without_text(#[case] source: &str) {
    assert_eq!(count_usage(source).prompt_tokens, 0);
}

fn embed_usage(source: &str, metadata: Option<&str>, resolved: &str) -> EmbeddingUsage {
    let response = metadata.map(json);
    embed_content_usage(
        &input(source),
        MODEL,
        response.as_ref(),
        &resolved_files(resolved),
        &CharacterCount,
    )
    .unwrap()
}

#[rstest]
#[case::absent(None)]
#[case::not_an_object(Some(r#""nope""#))]
#[case::wrong_count_type(Some(r#"{"promptTokenCount":"5"}"#))]
#[case::unknown_modality(Some(
    r#"{"promptTokenCount":5,"promptTokensDetails":[{"modality":"text","tokenCount":3}]}"#
))]
#[case::missing_detail_count(Some(
    r#"{"promptTokenCount":5,"promptTokensDetails":[{"modality":"TEXT"}]}"#
))]
#[case::mistyped_unused_field(Some(r#"{"promptTokenCount":5,"candidatesTokenCount":"bad"}"#))]
#[case::count_past_the_record_range(Some(r#"{"promptTokenCount":9223372036854775808}"#))]
fn invalid_metadata_falls_back_to_text_counting(#[case] metadata: Option<&str>) {
    let usage = embed_usage(r#""hello""#, metadata, "{}");
    assert_eq!(
        usage,
        EmbeddingUsage {
            prompt_tokens: 5,
            completion_tokens: 0,
            total_tokens: 5,
            prompt_tokens_details: None,
        }
    );
}

#[test]
fn invalid_metadata_falls_back_to_zero_for_media() {
    let usage = embed_usage(r#""data:image/png;base64,AAAA""#, None, "{}");
    assert_eq!(usage.prompt_tokens, 0);
    assert_eq!(usage.total_tokens, 0);
    assert_eq!(usage.prompt_tokens_details, None);
}

#[rstest]
#[case::prompt_only(r#"{"promptTokenCount":7}"#, 7, 7)]
#[case::total_zero(r#"{"promptTokenCount":7,"totalTokenCount":0}"#, 7, 7)]
#[case::total_null(r#"{"promptTokenCount":7,"totalTokenCount":null}"#, 7, 7)]
#[case::total_present(r#"{"promptTokenCount":7,"totalTokenCount":9}"#, 7, 9)]
#[case::prompt_null(r#"{"promptTokenCount":null}"#, 0, 0)]
#[case::unknown_key(r#"{"promptTokenCount":7,"futureCount":3}"#, 7, 7)]
fn reported_counts_carry_the_total_fallback(
    #[case] metadata: &str,
    #[case] prompt: u32,
    #[case] total: u32,
) {
    let usage = embed_usage(r#""hello""#, Some(metadata), "{}");
    assert_eq!(usage.prompt_tokens, prompt);
    assert_eq!(usage.total_tokens, total);
}

#[rstest]
#[case::inline(r#""data:image/png;base64,AAAA""#, "{}")]
#[case::gcs(r#""gs://bucket/x.JPG""#, "{}")]
#[case::resolved(
    r#""files/img""#,
    r#"{"files/img":{"mime_type":"image/png","uri":"https://example.com/files/img"}}"#
)]
#[case::nested(r#"[["data:image/png;base64,AA","data:image/jpeg;base64,BB"]]"#, "{}")]
fn image_only_inputs_report_the_prompt_as_image_tokens(
    #[case] source: &str,
    #[case] resolved: &str,
) {
    let usage = embed_usage(source, Some(r#"{"promptTokenCount":7}"#), resolved);
    assert_eq!(
        usage.prompt_tokens_details,
        Some(ModalityTokens {
            text: Some(0),
            audio: None,
            image: Some(7),
            video: None,
        })
    );
}

#[rstest]
#[case::text_beside_image(r#"["data:image/png;base64,AA","caption"]"#, "{}")]
#[case::unsupported_image_mime(r#""data:image/gif;base64,AA""#, "{}")]
#[case::other_media_kind(r#"["data:image/png;base64,AA","gs://bucket/x.pdf"]"#, "{}")]
#[case::http_url(r#""https://example.com/a.png""#, "{}")]
#[case::unresolved_file(r#""files/missing""#, "{}")]
#[case::resolved_non_image(
    r#""files/doc""#,
    r#"{"files/doc":{"mime_type":"application/pdf","uri":"https://example.com/files/doc"}}"#
)]
fn non_image_inputs_report_no_image_tokens(#[case] source: &str, #[case] resolved: &str) {
    let usage = embed_usage(source, Some(r#"{"promptTokenCount":7}"#), resolved);
    assert_eq!(
        usage.prompt_tokens_details,
        Some(ModalityTokens {
            text: Some(0),
            audio: None,
            image: Some(0),
            video: None,
        })
    );
}

#[test]
fn modality_details_sum_exact_uppercase_labels() {
    let usage = embed_usage(
        r#""hello""#,
        Some(
            r#"{"promptTokenCount":10,"promptTokensDetails":[
                {"modality":"TEXT","tokenCount":2},
                {"modality":"TEXT","tokenCount":3},
                {"modality":"IMAGE","tokenCount":4},
                {"modality":"AUDIO","tokenCount":1},
                {"modality":"VIDEO","tokenCount":6}
            ]}"#,
        ),
        "{}",
    );
    assert_eq!(
        usage.prompt_tokens_details,
        Some(ModalityTokens {
            text: Some(5),
            audio: Some(1),
            image: Some(4),
            video: Some(6),
        })
    );
}

#[test]
fn modality_details_default_missing_kinds_to_zero() {
    let usage = embed_usage(
        r#""hello""#,
        Some(
            r#"{"promptTokenCount":3,"promptTokensDetails":[{"modality":"TEXT","tokenCount":3}]}"#,
        ),
        "{}",
    );
    assert_eq!(
        usage.prompt_tokens_details,
        Some(ModalityTokens {
            text: Some(3),
            audio: Some(0),
            image: Some(0),
            video: Some(0),
        })
    );
}

#[test]
fn an_empty_detail_list_takes_the_no_details_path() {
    let usage = embed_usage(
        r#""data:image/png;base64,AA""#,
        Some(r#"{"promptTokenCount":4,"promptTokensDetails":[]}"#),
        "{}",
    );
    assert_eq!(
        usage.prompt_tokens_details,
        Some(ModalityTokens {
            text: Some(0),
            audio: None,
            image: Some(4),
            video: None,
        })
    );
}

#[rstest]
#[case::negative(r#"{"promptTokenCount":-1}"#, -1)]
fn counts_outside_the_range_are_errors(#[case] metadata: &str, #[case] value: i64) {
    let response = json(metadata);
    let error = embed_content_usage(
        &input(r#""hello""#),
        MODEL,
        Some(&response),
        &resolved_files("{}"),
        &CharacterCount,
    )
    .unwrap_err();
    assert!(matches!(error, GeminiEmbeddingError::TokenRange { value: found } if found == value));
}

#[test]
fn modality_sums_that_overflow_are_errors() {
    let response = json(
        r#"{"promptTokenCount":1,"promptTokensDetails":[
            {"modality":"TEXT","tokenCount":9223372036854775807},
            {"modality":"TEXT","tokenCount":1}
        ]}"#,
    );
    let error = embed_content_usage(
        &input(r#""hello""#),
        MODEL,
        Some(&response),
        &resolved_files("{}"),
        &CharacterCount,
    )
    .unwrap_err();
    assert!(matches!(error, GeminiEmbeddingError::TokenOverflow));
}

#[test]
fn a_failing_counter_keeps_its_cause() {
    let error = batch_usage(&input(r#""hello""#), MODEL, &FailingCount).unwrap_err();
    assert!(matches!(error, GeminiEmbeddingError::Counting { .. }));
}
