#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::{PromptTokensDetails, Usage};
use rstest::rstest;

use crate::gemini_usage::error::UsageError;
use crate::gemini_usage::metadata::UsageMetadata;
use crate::gemini_usage::usage::{apply_grounding_counters, calculate_usage};

fn metadata(source: &str) -> UsageMetadata {
    serde_json::from_str(source).unwrap()
}

fn json_list(source: &str) -> Vec<Json> {
    serde_json::from_str(source).unwrap()
}

fn calculated(usage_metadata: &str) -> Usage {
    calculate_usage(Some(&metadata(usage_metadata)), &[]).unwrap()
}

fn calculated_with(usage_metadata: &str, candidate_json: &str) -> Usage {
    calculate_usage(Some(&metadata(usage_metadata)), &json_list(candidate_json)).unwrap()
}

fn prompt_details(usage: &Usage) -> &PromptTokensDetails {
    usage.prompt_tokens_details.as_ref().unwrap()
}

#[test]
fn missing_usage_metadata_is_an_error() {
    let outcome = calculate_usage(None, &[]);
    assert!(matches!(outcome, Err(UsageError::MissingUsageMetadata)));
}

#[test]
fn inclusive_thinking_keeps_the_reported_candidate_count() {
    let usage = calculated(
        r#"{"promptTokenCount": 10, "candidatesTokenCount": 5, "toolUsePromptTokenCount": 3,
            "totalTokenCount": 18, "thoughtsTokenCount": 4}"#,
    );
    assert_eq!(usage.prompt_tokens, 13);
    assert_eq!(usage.completion_tokens, 5);
    assert_eq!(usage.total_tokens, 18);
    let completion = usage.completion_tokens_details.unwrap();
    assert_eq!(completion.text_tokens, Some(5));
    assert_eq!(completion.reasoning_tokens, Some(4));
}

#[test]
fn inclusive_thinking_counts_tool_tokens_grounding_does_not_bill() {
    let usage = calculated_with(
        r#"{"promptTokenCount": 10, "candidatesTokenCount": 5, "toolUsePromptTokenCount": 3,
            "totalTokenCount": 18, "thoughtsTokenCount": 4}"#,
        r#"[{"groundingMetadata": {"webSearchQueries": ["q"]}}]"#,
    );
    assert_eq!(usage.prompt_tokens, 10);
    assert_eq!(usage.completion_tokens, 5);
    assert_eq!(
        usage.completion_tokens_details.unwrap().reasoning_tokens,
        Some(4)
    );
}

#[test]
fn exclusive_thinking_adds_reasoning_to_the_completion_count() {
    let usage = calculated(
        r#"{"promptTokenCount": 10, "candidatesTokenCount": 5, "toolUsePromptTokenCount": 3,
            "totalTokenCount": 19, "thoughtsTokenCount": 4}"#,
    );
    assert_eq!(usage.prompt_tokens, 13);
    assert_eq!(usage.completion_tokens, 9);
    assert_eq!(
        usage.completion_tokens_details.unwrap().text_tokens,
        Some(5)
    );
}

#[test]
fn zero_thoughts_leave_the_text_detail_absent() {
    let usage =
        calculated(r#"{"promptTokenCount": 1, "totalTokenCount": 5, "thoughtsTokenCount": 0}"#);
    assert_eq!(usage.completion_tokens, 0);
    let completion = usage.completion_tokens_details.unwrap();
    assert_eq!(completion.text_tokens, None);
    assert_eq!(completion.reasoning_tokens, Some(0));
}

#[test]
fn explicit_empty_cache_details_disable_the_implicit_cache_subtraction() {
    let usage = calculated(
        r#"{"promptTokenCount": 10, "candidatesTokenCount": 0, "totalTokenCount": 10,
            "cachedContentTokenCount": 4,
            "promptTokensDetails": [{"modality": "TEXT", "tokenCount": 10}],
            "cacheTokensDetails": []}"#,
    );
    let details = prompt_details(&usage);
    assert_eq!(details.text_tokens, Some(10));
    assert_eq!(details.cached_tokens, Some(4));
    assert!(usage.completion_tokens_details.is_none());
}

#[test]
fn implicit_cache_subtracts_from_the_prompt_text() {
    let usage = calculated(
        r#"{"promptTokenCount": 10, "candidatesTokenCount": 0, "totalTokenCount": 10,
            "cachedContentTokenCount": 4,
            "promptTokensDetails": [{"modality": "text", "tokenCount": 10}]}"#,
    );
    let details = prompt_details(&usage);
    assert_eq!(details.text_tokens, Some(6));
    assert_eq!(details.cached_tokens, Some(4));
}

#[test]
fn implicit_cache_without_a_prompt_text_detail_leaves_the_count_alone() {
    let usage = calculated(
        r#"{"promptTokenCount": 10, "candidatesTokenCount": 0, "totalTokenCount": 10,
            "cachedContentTokenCount": 4}"#,
    );
    let details = prompt_details(&usage);
    assert_eq!(details.text_tokens, None);
    assert_eq!(details.cached_tokens, Some(4));
}

#[test]
fn document_details_fold_into_text_and_accumulate_together() {
    let usage = calculated(
        r#"{"promptTokenCount": 12, "candidatesTokenCount": 7, "totalTokenCount": 19,
            "promptTokensDetails": [{"modality": "DOCUMENT", "tokenCount": 5}, {"modality": "text", "tokenCount": 7}],
            "cacheTokensDetails": [{"modality": "document", "tokenCount": 2}, {"modality": "TEXT", "tokenCount": 1}],
            "candidatesTokensDetails": [
                {"modality": "DOCUMENT", "tokenCount": 2},
                {"modality": "TEXT", "tokenCount": 3},
                {"modality": "IMAGE", "tokenCount": 2}
            ]}"#,
    );
    assert_eq!(usage.completion_tokens, 7);
    let details = prompt_details(&usage);
    assert_eq!(details.text_tokens, Some(9));
    assert_eq!(details.cached_tokens, None);
    let completion = usage.completion_tokens_details.unwrap();
    assert_eq!(completion.text_tokens, Some(5));
    assert_eq!(completion.image_tokens, Some(2));
}

#[rstest]
#[case::upper("TEXT")]
#[case::lower("text")]
#[case::mixed("TexT")]
fn modality_names_are_case_insensitive(#[case] modality: &str) {
    let source = format!(
        r#"{{"promptTokenCount": 6, "totalTokenCount": 6,
             "promptTokensDetails": [{{"modality": "{modality}", "tokenCount": 6}}]}}"#
    );
    let usage = calculated(&source);
    assert_eq!(prompt_details(&usage).text_tokens, Some(6));
}

#[rstest]
#[case::camel_float(
    r#"{"modality": "TEXT", "tokenCount": 1.5, "token_count": 4}"#,
    Some(0)
)]
#[case::camel_text(
    r#"{"modality": "TEXT", "tokenCount": "3", "token_count": 4}"#,
    Some(0)
)]
#[case::snake_only(r#"{"modality": "TEXT", "token_count": 4}"#, Some(4))]
#[case::absent("", None)]
#[case::null_camel(
    r#"{"modality": "TEXT", "tokenCount": null, "token_count": 4}"#,
    Some(4)
)]
fn token_count_prefers_the_camel_key_and_ignores_noninteger_values(
    #[case] detail: &str,
    #[case] expected: Option<i64>,
) {
    let entries = if detail.is_empty() {
        String::from("[]")
    } else {
        format!("[{detail}]")
    };
    let source = format!(
        r#"{{"promptTokenCount": 1, "totalTokenCount": 1, "promptTokensDetails": {entries}}}"#
    );
    let usage = calculated(&source);
    assert_eq!(prompt_details(&usage).text_tokens, expected);
}

#[test]
fn missing_modality_accumulates_into_no_counter() {
    let usage = calculated(
        r#"{"promptTokenCount": 1, "totalTokenCount": 1,
            "promptTokensDetails": [{"tokenCount": 5}]}"#,
    );
    let details = prompt_details(&usage);
    assert_eq!(details.text_tokens, None);
    assert_eq!(details.audio_tokens, None);
    assert_eq!(details.image_tokens, None);
    assert_eq!(details.video_tokens, None);
}

#[test]
fn unknown_modalities_are_ignored() {
    let usage = calculated(
        r#"{"promptTokenCount": 1, "totalTokenCount": 1,
            "promptTokensDetails": [{"modality": "CHEMICAL", "tokenCount": 5}, {"modality": "VIDEO", "tokenCount": 2}]}"#,
    );
    let details = prompt_details(&usage);
    assert_eq!(details.video_tokens, Some(2));
    assert_eq!(details.text_tokens, None);
}

#[test]
fn cached_modality_counts_subtract_from_their_own_prompt_counters() {
    let usage = calculated(
        r#"{"promptTokenCount": 22, "totalTokenCount": 22,
            "promptTokensDetails": [
                {"modality": "TEXT", "tokenCount": 10},
                {"modality": "AUDIO", "tokenCount": 5},
                {"modality": "IMAGE", "tokenCount": 4},
                {"modality": "VIDEO", "tokenCount": 3}
            ],
            "cacheTokensDetails": [
                {"modality": "TEXT", "tokenCount": 2},
                {"modality": "AUDIO", "tokenCount": 1},
                {"modality": "IMAGE", "tokenCount": 1},
                {"modality": "VIDEO", "tokenCount": 5}
            ]}"#,
    );
    let details = prompt_details(&usage);
    assert_eq!(details.text_tokens, Some(8));
    assert_eq!(details.audio_tokens, Some(4));
    assert_eq!(details.image_tokens, Some(3));
    assert_eq!(details.video_tokens, Some(-2));
}

#[test]
fn response_and_candidate_details_accumulate_together() {
    let usage = calculated(
        r#"{"promptTokenCount": 8, "candidatesTokenCount": 10, "totalTokenCount": 20,
            "responseTokenCount": 12,
            "responseTokensDetails": [
                {"modality": "TEXT", "tokenCount": 4},
                {"modality": "AUDIO", "tokenCount": 1},
                {"modality": "DOCUMENT", "tokenCount": 1},
                {"modality": "IMAGE", "tokenCount": 9}
            ],
            "candidatesTokensDetails": [
                {"modality": "TEXT", "tokenCount": 3},
                {"modality": "AUDIO", "tokenCount": 2},
                {"modality": "IMAGE", "tokenCount": 2},
                {"modality": "VIDEO", "tokenCount": 1}
            ]}"#,
    );
    assert_eq!(usage.completion_tokens, 12);
    let completion = usage.completion_tokens_details.unwrap();
    assert_eq!(completion.text_tokens, Some(8));
    assert_eq!(completion.audio_tokens, Some(3));
    assert_eq!(completion.image_tokens, Some(2));
    assert_eq!(completion.video_tokens, Some(1));
    assert_eq!(completion.reasoning_tokens, None);
}

#[test]
fn an_absent_text_detail_is_inferred_without_clamping() {
    let usage = calculated(
        r#"{"promptTokenCount": 1, "candidatesTokenCount": 3, "totalTokenCount": 4,
            "candidatesTokensDetails": [{"modality": "IMAGE", "tokenCount": 5}]}"#,
    );
    assert_eq!(usage.completion_tokens, 3);
    let completion = usage.completion_tokens_details.unwrap();
    assert_eq!(completion.text_tokens, Some(-2));
    assert_eq!(completion.image_tokens, Some(5));
}

#[test]
fn an_absent_text_detail_is_inferred_from_the_other_modalities() {
    let usage = calculated(
        r#"{"promptTokenCount": 1, "candidatesTokenCount": 10, "totalTokenCount": 11,
            "candidatesTokensDetails": [
                {"modality": "IMAGE", "tokenCount": 3},
                {"modality": "AUDIO", "tokenCount": 2},
                {"modality": "VIDEO", "tokenCount": 1}
            ]}"#,
    );
    assert_eq!(
        usage.completion_tokens_details.unwrap().text_tokens,
        Some(4)
    );
}

#[test]
fn an_explicit_zero_text_detail_blocks_the_inference() {
    let usage = calculated(
        r#"{"promptTokenCount": 1, "candidatesTokenCount": 10, "totalTokenCount": 11,
            "candidatesTokensDetails": [
                {"modality": "TEXT", "tokenCount": 0},
                {"modality": "IMAGE", "tokenCount": 5}
            ]}"#,
    );
    assert_eq!(
        usage.completion_tokens_details.unwrap().text_tokens,
        Some(0)
    );
}

#[test]
fn an_empty_response_details_list_still_creates_the_completion_details() {
    let usage = calculated(
        r#"{"promptTokenCount": 1, "candidatesTokenCount": 0, "totalTokenCount": 1,
            "responseTokensDetails": []}"#,
    );
    let completion = usage.completion_tokens_details.unwrap();
    assert_eq!(completion.text_tokens, None);
    assert_eq!(completion.reasoning_tokens, None);
}

#[test]
fn an_inclusive_thinking_reply_clamps_the_constructor_text_at_zero() {
    let usage =
        calculated(r#"{"promptTokenCount": 1, "totalTokenCount": 1, "thoughtsTokenCount": 5}"#);
    assert_eq!(usage.completion_tokens, 0);
    let completion = usage.completion_tokens_details.unwrap();
    assert_eq!(completion.reasoning_tokens, Some(5));
    assert_eq!(completion.text_tokens, Some(0));
}

#[test]
fn the_constructor_clamp_subtracts_image_tokens() {
    let usage = calculated(
        r#"{"promptTokenCount": 1, "totalTokenCount": 1, "thoughtsTokenCount": 5,
            "responseTokenCount": 7,
            "candidatesTokensDetails": [{"modality": "IMAGE", "tokenCount": 2}]}"#,
    );
    assert_eq!(usage.completion_tokens, 7);
    let completion = usage.completion_tokens_details.unwrap();
    assert_eq!(completion.text_tokens, Some(0));
    assert_eq!(completion.image_tokens, Some(2));
    assert_eq!(completion.reasoning_tokens, Some(5));
}

#[test]
fn the_constructor_clamp_subtracts_audio_tokens() {
    let usage = calculated(
        r#"{"promptTokenCount": 1, "totalTokenCount": 1, "thoughtsTokenCount": 5,
            "responseTokenCount": 7,
            "responseTokensDetails": [{"modality": "AUDIO", "tokenCount": 2}]}"#,
    );
    assert_eq!(usage.completion_tokens, 7);
    let completion = usage.completion_tokens_details.unwrap();
    assert_eq!(completion.text_tokens, Some(0));
    assert_eq!(completion.audio_tokens, Some(2));
    assert_eq!(completion.reasoning_tokens, Some(5));
}

#[test]
fn a_candidate_detail_list_creates_the_completion_details_without_a_candidate_count() {
    let usage = calculated(
        r#"{"promptTokenCount": 1, "totalTokenCount": 1,
            "candidatesTokensDetails": [{"modality": "IMAGE", "tokenCount": 0}]}"#,
    );
    assert_eq!(usage.completion_tokens, 0);
    let completion = usage.completion_tokens_details.unwrap();
    assert_eq!(completion.text_tokens, None);
    assert_eq!(completion.image_tokens, Some(0));
    assert_eq!(completion.reasoning_tokens, None);
}

#[test]
fn a_zero_response_token_count_falls_back_to_the_candidate_count() {
    let usage = calculated(
        r#"{"promptTokenCount": 1, "candidatesTokenCount": 5, "totalTokenCount": 6,
            "responseTokenCount": 0}"#,
    );
    assert_eq!(usage.completion_tokens, 5);
}

#[test]
fn tool_use_tokens_bill_the_prompt_without_grounding() {
    let usage = calculated(
        r#"{"promptTokenCount": 10, "candidatesTokenCount": 2, "totalTokenCount": 15,
            "toolUsePromptTokenCount": 3}"#,
    );
    assert_eq!(usage.prompt_tokens, 13);
    let details = prompt_details(&usage);
    assert_eq!(details.tool_use_tokens, Some(3));
    assert_eq!(details.web_search_requests, None);
    assert_eq!(details.google_maps_grounding_requests, None);
}

#[test]
fn url_context_grounding_bills_tool_use_tokens() {
    let usage = calculated_with(
        r#"{"promptTokenCount": 10, "candidatesTokenCount": 2, "totalTokenCount": 15,
            "toolUsePromptTokenCount": 3}"#,
        r#"[{"groundingMetadata": {"groundingChunks": [{"web": {"uri": "https://example.com"}}]}}]"#,
    );
    assert_eq!(usage.prompt_tokens, 13);
    assert_eq!(prompt_details(&usage).tool_use_tokens, Some(3));
}

#[test]
fn a_web_search_excludes_billable_tool_tokens_from_the_prompt() {
    let usage = calculated_with(
        r#"{"promptTokenCount": 10, "candidatesTokenCount": 2, "totalTokenCount": 15,
            "toolUsePromptTokenCount": 3}"#,
        r#"[{"groundingMetadata": {"webSearchQueries": ["q"], "groundingChunks": [{"web": {}}]}}]"#,
    );
    assert_eq!(usage.prompt_tokens, 10);
    assert_eq!(prompt_details(&usage).tool_use_tokens, Some(3));
}

#[test]
fn a_maps_lookup_excludes_billable_tool_tokens_from_the_prompt() {
    let usage = calculated_with(
        r#"{"promptTokenCount": 10, "candidatesTokenCount": 2, "totalTokenCount": 15,
            "toolUsePromptTokenCount": 3}"#,
        r#"[{"groundingMetadata": {"groundingChunks": [{"maps": {}}]}}]"#,
    );
    assert_eq!(usage.prompt_tokens, 10);
}

#[test]
fn a_scalar_grounding_metadata_on_a_candidate_is_consumed() {
    let usage = calculated_with(
        r#"{"promptTokenCount": 10, "candidatesTokenCount": 2, "totalTokenCount": 15,
            "toolUsePromptTokenCount": 3}"#,
        r#"[{"groundingMetadata": {"webSearchQueries": ["q"]}}]"#,
    );
    assert_eq!(usage.prompt_tokens, 10);
}

#[test]
fn a_non_object_candidate_carries_no_grounding() {
    let usage = calculated_with(
        r#"{"promptTokenCount": 10, "candidatesTokenCount": 2, "totalTokenCount": 15,
            "toolUsePromptTokenCount": 3}"#,
        r#"[5, "webSearchQueries"]"#,
    );
    assert_eq!(usage.prompt_tokens, 13);
}

#[test]
fn an_explicit_zero_tool_use_count_leaves_the_detail_unset() {
    let usage = calculated(
        r#"{"promptTokenCount": 10, "candidatesTokenCount": 2, "totalTokenCount": 12,
            "toolUsePromptTokenCount": 0}"#,
    );
    assert_eq!(usage.prompt_tokens, 10);
    assert_eq!(prompt_details(&usage).tool_use_tokens, None);
}

#[test]
fn the_reported_total_never_follows_the_component_sum() {
    let usage =
        calculated(r#"{"promptTokenCount": 3, "candidatesTokenCount": 4, "totalTokenCount": 100}"#);
    assert_eq!(usage.prompt_tokens, 3);
    assert_eq!(usage.completion_tokens, 4);
    assert_eq!(usage.total_tokens, 100);
}

#[test]
fn an_empty_usage_metadata_yields_zero_usage_with_an_empty_prompt_details_record() {
    let usage = calculated("{}");
    assert_eq!(usage.prompt_tokens, 0);
    assert_eq!(usage.completion_tokens, 0);
    assert_eq!(usage.total_tokens, 0);
    assert_eq!(
        usage.prompt_tokens_details,
        Some(PromptTokensDetails::default())
    );
    assert!(usage.completion_tokens_details.is_none());
    assert!(usage.numeric_extras.is_empty());
}

#[test]
fn grounding_counters_land_on_the_prompt_details() {
    let mut usage = calculated("{}");
    apply_grounding_counters(
        &mut usage,
        &json_list(
            r#"[{"webSearchQueries": ["w1", "w2"], "groundingChunks": [{"web": {}, "maps": {}}]},
                {"webSearchQueries": ["m1"], "groundingChunks": [{"maps": {}}], "googleMapsWidgetContextToken": "token"}]"#,
        ),
    );
    let details = prompt_details(&usage);
    assert_eq!(details.web_search_requests, Some(2));
    assert_eq!(details.google_maps_grounding_requests, Some(1));
}

#[test]
fn a_later_empty_grounding_metadata_keeps_the_earlier_counters() {
    let mut usage = calculated("{}");
    apply_grounding_counters(&mut usage, &json_list(r#"[{"webSearchQueries": ["w"]}]"#));
    apply_grounding_counters(&mut usage, &[]);
    assert_eq!(prompt_details(&usage).web_search_requests, Some(1));
}

#[test]
fn grounding_counters_create_the_prompt_details_when_the_usage_has_none() {
    let mut usage = Usage::default();
    apply_grounding_counters(&mut usage, &json_list(r#"[{"webSearchQueries": ["w"]}]"#));
    assert_eq!(prompt_details(&usage).web_search_requests, Some(1));
}
