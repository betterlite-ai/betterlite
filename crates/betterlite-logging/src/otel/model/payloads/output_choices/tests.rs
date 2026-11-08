#![cfg(test)]

use super::{finish_reasons, output_choices};
use crate::otel::model::metadata::wire::MetadataMap;

fn parse(source: &str) -> MetadataMap {
    serde_json::from_str(source).unwrap()
}

fn parsed_choices(source: &str) -> Vec<MetadataMap> {
    serde_json::from_str(source).unwrap()
}

fn assert_choices(actual: &[MetadataMap], expected: &str) {
    assert_eq!(
        serde_json::to_string(actual).unwrap(),
        serde_json::to_string(&parsed_choices(expected)).unwrap()
    );
}

#[rstest::rstest]
#[case::empty("{}", "[]")]
#[case::completion_before_output(
    r#"{"choices":[{"text":"completed"}],"output":[{"type":"message","role":"user","content":[{"type":"output_text","text":"from output"}]}]}"#,
    r#"[{"message":{"role":"assistant","content":"completed","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::responses_before_pages(
    r#"{"output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"folded"}]}],"pages":[{"markdown":"scanned"}]}"#,
    r#"[{"message":{"role":"assistant","content":"folded","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::ocr_before_transcription(
    r#"{"pages":[{"markdown":"scanned"}],"text":"spoken"}"#,
    r#"[{"message":{"role":"assistant","content":"scanned","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::ocr_before_moderation(
    r#"{"pages":[{"markdown":"scanned"}],"results":[{"flagged":false}]}"#,
    r#"[{"message":{"role":"assistant","content":"scanned","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::transcription_before_moderation(
    r#"{"text":"spoken","results":[{"flagged":false}]}"#,
    r#"[{"message":{"role":"assistant","content":"spoken","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::moderation_before_rerank(
    r#"{"results":[{"flagged":false,"index":0,"relevance_score":0.5}]}"#,
    r#"[{"message":{"role":"assistant","content":"not flagged","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::rerank_before_search(
    r#"{"results":[{"index":0,"relevance_score":0.5,"title":"Doc"}]}"#,
    r#"[{"message":{"role":"assistant","content":"[0] 0.5","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::search_before_image(
    r#"{"results":[{"title":"Doc"}],"data":[{"url":"https://example.com/a.png"}]}"#,
    r#"[{"message":{"role":"assistant","content":"Doc","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::image_before_binary(
    r#"{"data":[{"url":"https://example.com/a.png"}],"num_bytes":3}"#,
    r#"[{"message":{"role":"assistant","content":"https://example.com/a.png","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::binary_last(
    r#"{"num_bytes":3}"#,
    r#"[{"message":{"role":"assistant","content":"3 bytes","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::non_dict_choices_fall_through(
    r#"{"choices":[5],"pages":[{"markdown":"scanned"}]}"#,
    r#"[{"message":{"role":"assistant","content":"scanned","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
fn first_non_empty_reader_wins(#[case] response: &str, #[case] expected: &str) {
    assert_choices(&output_choices(&parse(response)), expected);
}

#[rstest::rstest]
#[case::chat_passthrough(
    r#"{"choices":[{"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}]}"#,
    r#"[{"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}]"#
)]
#[case::text_rewritten(
    r#"{"choices":[{"text":"hi","index":0,"logprobs":null,"finish_reason":"length"}]}"#,
    r#"[{"index":0,"logprobs":null,"message":{"role":"assistant","content":"hi","refusal":null,"tool_calls":null},"finish_reason":"length"}]"#
)]
#[case::no_finish_reason(
    r#"{"choices":[{"text":"hi"}]}"#,
    r#"[{"message":{"role":"assistant","content":"hi","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::null_message_passthrough(
    r#"{"choices":[{"message":null,"text":"hi"}]}"#,
    r#"[{"message":null,"text":"hi"}]"#
)]
#[case::non_string_text_passthrough(
    r#"{"choices":[{"text":5,"index":0}]}"#,
    r#"[{"text":5,"index":0}]"#
)]
#[case::non_string_finish_reason(
    r#"{"choices":[{"text":"hi","finish_reason":3}]}"#,
    r#"[{"message":{"role":"assistant","content":"hi","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::empty_choices("{}", "[]")]
fn completion_choices_match_litellm(#[case] response: &str, #[case] expected: &str) {
    assert_choices(&output_choices(&parse(response)), expected);
}

#[rstest::rstest]
#[case::text_stop(
    r#"{"output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Hello"}]}],"status":"completed"}"#,
    r#"[{"message":{"role":"assistant","content":"Hello","refusal":null,"tool_calls":null},"finish_reason":"stop"}]"#
)]
#[case::multiple_parts(
    r#"{"output":[{"type":"message","role":"user","content":[{"type":"output_text","text":"Hello"},{"type":"output_text","text":" world"}]}],"status":"completed"}"#,
    r#"[{"message":{"role":"user","content":"Hello world","refusal":null,"tool_calls":null},"finish_reason":"stop"}]"#
)]
#[case::empty_text_part(
    r#"{"output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":""}]}],"status":"completed"}"#,
    r#"[{"message":{"role":"assistant","content":"","refusal":null,"tool_calls":null},"finish_reason":"stop"}]"#
)]
#[case::other_part_types_skipped(
    r#"{"output":[{"type":"message","role":"assistant","content":[{"type":"input_text","text":"ignored"},{"type":"output_text","text":"kept"}]}],"status":"completed"}"#,
    r#"[{"message":{"role":"assistant","content":"kept","refusal":null,"tool_calls":null},"finish_reason":"stop"}]"#
)]
#[case::refusal(
    r#"{"output":[{"type":"message","role":"assistant","content":[{"type":"refusal","refusal":"No"}]}],"status":"completed"}"#,
    r#"[{"message":{"role":"assistant","content":null,"refusal":"No","tool_calls":null},"finish_reason":"stop"}]"#
)]
#[case::text_and_refusal(
    r#"{"output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Hi"},{"type":"refusal","refusal":"No"}]}],"status":"completed"}"#,
    r#"[{"message":{"role":"assistant","content":"Hi","refusal":"No","tool_calls":null},"finish_reason":"stop"}]"#
)]
#[case::function_call(
    r#"{"output":[{"type":"function_call","id":"fc_1","name":"lookup","arguments":"{\"x\":1}"}],"status":"completed"}"#,
    r#"[{"message":{"role":"assistant","content":null,"refusal":null,"tool_calls":[{"id":"fc_1","type":"function","function":{"name":"lookup","arguments":"{\"x\":1}"}}]},"finish_reason":"tool_calls"}]"#
)]
#[case::custom_tool_call(
    r#"{"output":[{"type":"custom_tool_call","call_id":"ctc_1","id":"ignored","name":"run","input":"raw"}],"status":"completed"}"#,
    r#"[{"message":{"role":"assistant","content":null,"refusal":null,"tool_calls":[{"id":"ctc_1","type":"function","function":{"name":"run","arguments":"raw"}}]},"finish_reason":"tool_calls"}]"#
)]
#[case::empty_call_id_falls_back_to_id(
    r#"{"output":[{"type":"function_call","call_id":"","id":"fc_2","name":"n","arguments":"a"}],"status":"in_progress"}"#,
    r#"[{"message":{"role":"assistant","content":null,"refusal":null,"tool_calls":[{"id":"fc_2","type":"function","function":{"name":"n","arguments":"a"}}]},"finish_reason":null}]"#
)]
#[case::missing_tool_call_fields(
    r#"{"output":[{"type":"function_call"}]}"#,
    r#"[{"message":{"role":"assistant","content":null,"refusal":null,"tool_calls":[{"id":"","type":"function","function":{"name":"","arguments":""}}]},"finish_reason":null}]"#
)]
#[case::no_role(
    r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"Hi"}]}],"status":"completed"}"#,
    r#"[{"message":{"role":"assistant","content":"Hi","refusal":null,"tool_calls":null},"finish_reason":"stop"}]"#
)]
#[case::first_role_wins(
    r#"{"output":[{"type":"message","content":[]},{"type":"message","role":"user","content":[{"type":"output_text","text":"Hi"}]}],"status":"completed"}"#,
    r#"[{"message":{"role":"user","content":"Hi","refusal":null,"tool_calls":null},"finish_reason":"stop"}]"#
)]
#[case::incomplete_length(
    r#"{"output":[{"type":"message","role":"assistant","content":[]}],"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"}}"#,
    r#"[{"message":{"role":"assistant","content":null,"refusal":null,"tool_calls":null},"finish_reason":"length"}]"#
)]
#[case::incomplete_content_filter(
    r#"{"output":[{"type":"message","role":"assistant","content":[]}],"status":"incomplete","incomplete_details":{"reason":"content_filter"}}"#,
    r#"[{"message":{"role":"assistant","content":null,"refusal":null,"tool_calls":null},"finish_reason":"content_filter"}]"#
)]
#[case::incomplete_without_details(
    r#"{"output":[{"type":"message","role":"assistant","content":[]}],"status":"incomplete"}"#,
    r#"[{"message":{"role":"assistant","content":null,"refusal":null,"tool_calls":null},"finish_reason":"length"}]"#
)]
#[case::in_progress(
    r#"{"output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Hi"}]}],"status":"in_progress"}"#,
    r#"[{"message":{"role":"assistant","content":"Hi","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::refusal_with_tool_call(
    r#"{"output":[{"type":"message","role":"assistant","content":[{"type":"refusal","refusal":"No"}]},{"type":"custom_tool_call","call_id":"c1","name":"run","input":"x"}],"status":"completed"}"#,
    r#"[{"message":{"role":"assistant","content":null,"refusal":"No","tool_calls":[{"id":"c1","type":"function","function":{"name":"run","arguments":"x"}}]},"finish_reason":"tool_calls"}]"#
)]
#[case::reasoning_only(
    r#"{"output":[{"type":"reasoning","summary":[]}],"status":"completed"}"#,
    "[]"
)]
fn responses_output_folds_into_one_choice(#[case] response: &str, #[case] expected: &str) {
    assert_choices(&output_choices(&parse(response)), expected);
}

#[rstest::rstest]
#[case::ocr_pages(
    r##"{"pages":[{"markdown":"# Page 1"},{"markdown":"Page 2"}]}"##,
    r##"[{"message":{"role":"assistant","content":"# Page 1\n\nPage 2","refusal":null,"tool_calls":null},"finish_reason":null}]"##
)]
#[case::ocr_empty_markdown(
    r#"{"pages":[{"markdown":""}]}"#,
    r#"[{"message":{"role":"assistant","content":"","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::ocr_null_markdown_skipped(r#"{"pages":[{"markdown":null}]}"#, "[]")]
#[case::ocr_missing_pages("{}", "[]")]
#[case::transcription_text(
    r#"{"text":"spoken words"}"#,
    r#"[{"message":{"role":"assistant","content":"spoken words","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::transcription_empty_text(r#"{"text":""}"#, "[]")]
#[case::transcription_null_text(r#"{"text":null}"#, "[]")]
#[case::transcription_missing_text("{}", "[]")]
fn ocr_and_transcription_text(#[case] response: &str, #[case] expected: &str) {
    assert_choices(&output_choices(&parse(response)), expected);
}

#[rstest::rstest]
#[case::moderation_not_flagged(
    r#"{"results":[{"flagged":false}]}"#,
    r#"[{"message":{"role":"assistant","content":"not flagged","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::moderation_flagged_categories(
    r#"{"results":[{"flagged":true,"categories":{"harassment":true,"hate":true,"violence":false}}]}"#,
    r#"[{"message":{"role":"assistant","content":"flagged: harassment, hate","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::moderation_flagged_no_hits(
    r#"{"results":[{"flagged":true,"categories":{"hate":false}}]}"#,
    r#"[{"message":{"role":"assistant","content":"flagged","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::moderation_without_categories(
    r#"{"results":[{"flagged":true}]}"#,
    r#"[{"message":{"role":"assistant","content":"flagged","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::moderation_multiple_results(
    r#"{"results":[{"flagged":false},{"flagged":true,"categories":{}}]}"#,
    r#"[{"message":{"role":"assistant","content":"not flagged\n\nflagged","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::non_bool_flagged_falls_to_rerank(
    r#"{"results":[{"flagged":1,"index":0,"relevance_score":1.0}]}"#,
    r#"[{"message":{"role":"assistant","content":"[0] 1.0","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::rerank_with_document(
    r#"{"results":[{"index":0,"relevance_score":0.5,"document":{"text":"doc"}}]}"#,
    r#"[{"message":{"role":"assistant","content":"[0] 0.5\ndoc","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::rerank_without_document(
    r#"{"results":[{"index":2,"relevance_score":0.25}]}"#,
    r#"[{"message":{"role":"assistant","content":"[2] 0.25","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::rerank_empty_document_text(
    r#"{"results":[{"index":1,"relevance_score":0.5,"document":{"text":""}}]}"#,
    r#"[{"message":{"role":"assistant","content":"[1] 0.5","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::rerank_whole_score(
    r#"{"results":[{"index":0,"relevance_score":1.0}]}"#,
    r#"[{"message":{"role":"assistant","content":"[0] 1.0","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::rerank_missing_score_falls_to_search(
    r#"{"results":[{"index":0},{"title":"Doc"}]}"#,
    r#"[{"message":{"role":"assistant","content":"Doc","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::rerank_non_numeric_score_skipped(
    r#"{"results":[{"index":0,"relevance_score":"high"}]}"#,
    "[]"
)]
#[case::search_all_fields(
    r#"{"results":[{"title":"Title","url":"https://example.com","snippet":"Snippet"}]}"#,
    r#"[{"message":{"role":"assistant","content":"Title\nhttps://example.com\nSnippet","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::search_partial_fields(
    r#"{"results":[{"title":"Title","url":""}]}"#,
    r#"[{"message":{"role":"assistant","content":"Title","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::search_blank_result_skipped(r#"{"results":[{"title":"","url":""}]}"#, "[]")]
#[case::search_multiple_results(
    r#"{"results":[{"title":"one"},{"url":"https://two"}]}"#,
    r#"[{"message":{"role":"assistant","content":"one\n\nhttps://two","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
fn moderation_rerank_and_search_lines(#[case] response: &str, #[case] expected: &str) {
    assert_choices(&output_choices(&parse(response)), expected);
}

#[rstest::rstest]
#[case::image_url(
    r#"{"data":[{"url":"https://example.com/a.png"}]}"#,
    r#"[{"message":{"role":"assistant","content":"https://example.com/a.png","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::image_empty_url(
    r#"{"data":[{"url":""}]}"#,
    r#"[{"message":{"role":"assistant","content":"","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::image_b64_padding(
    r#"{"data":[{"b64_json":"aGVsbG8="}]}"#,
    r#"[{"message":{"role":"assistant","content":"b64_json image (5 bytes)","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::image_b64_single_pad_sign(
    r#"{"data":[{"b64_json":"="}]}"#,
    r#"[{"message":{"role":"assistant","content":"b64_json image (-1 bytes)","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::image_b64_without_padding(
    r#"{"data":[{"b64_json":"aGVsbG8"}]}"#,
    r#"[{"message":{"role":"assistant","content":"b64_json image (5 bytes)","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::image_null_url_uses_b64(
    r#"{"data":[{"url":null,"b64_json":"aGVsbG8=","revised_prompt":null}]}"#,
    r#"[{"message":{"role":"assistant","content":"b64_json image (5 bytes)","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::image_empty_url_beats_b64(
    r#"{"data":[{"url":"","b64_json":"aGVsbG8="}]}"#,
    r#"[{"message":{"role":"assistant","content":"","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::image_revised_prompt(
    r#"{"data":[{"url":"https://example.com/a.png","revised_prompt":"better"}]}"#,
    r#"[{"message":{"role":"assistant","content":"better\nhttps://example.com/a.png","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::image_empty_revised_prompt(
    r#"{"data":[{"url":"u","revised_prompt":""}]}"#,
    r#"[{"message":{"role":"assistant","content":"u","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::image_no_location(r#"{"data":[{"revised_prompt":"better"}]}"#, "[]")]
#[case::image_non_string_b64(r#"{"data":[{"b64_json":5}]}"#, "[]")]
#[case::image_multiple(
    r#"{"data":[{"url":"a"},{"url":"b"}]}"#,
    r#"[{"message":{"role":"assistant","content":"a\n\nb","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::binary_with_content_type(
    r#"{"num_bytes":12,"content_type":"application/octet-stream"}"#,
    r#"[{"message":{"role":"assistant","content":"application/octet-stream (12 bytes)","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::binary_without_content_type(
    r#"{"num_bytes":12}"#,
    r#"[{"message":{"role":"assistant","content":"12 bytes","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::binary_empty_content_type(
    r#"{"num_bytes":12,"content_type":""}"#,
    r#"[{"message":{"role":"assistant","content":"12 bytes","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::binary_zero_bytes(
    r#"{"num_bytes":0}"#,
    r#"[{"message":{"role":"assistant","content":"0 bytes","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::binary_string_size(
    r#"{"num_bytes":"7"}"#,
    r#"[{"message":{"role":"assistant","content":"7 bytes","refusal":null,"tool_calls":null},"finish_reason":null}]"#
)]
#[case::binary_missing_size("{}", "[]")]
fn media_becomes_size_summaries(#[case] response: &str, #[case] expected: &str) {
    assert_choices(&output_choices(&parse(response)), expected);
}

#[rstest::rstest]
#[case::filtered(
    r#"[{"finish_reason":"stop"},{"finish_reason":""},{"finish_reason":null},{},{"finish_reason":"length"}]"#,
    &["stop", "length"]
)]
#[case::empty_choices("[]", &[])]
#[case::all_empty(r#"[{"finish_reason":""},{"finish_reason":null},{}]"#, &[])]
#[case::non_string_dropped(r#"[{"finish_reason":3}]"#, &[])]
fn finish_reasons_keep_non_empty_values(#[case] choices: &str, #[case] expected: &[&str]) {
    assert_eq!(finish_reasons(&parsed_choices(choices)), expected);
}
