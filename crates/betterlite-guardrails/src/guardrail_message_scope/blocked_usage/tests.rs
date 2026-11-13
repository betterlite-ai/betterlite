#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::{
    BlockedUsage, OriginalResponse, ResponsesUsage, anthropic_stream_usage,
    blocked_chat_stream_usage, blocked_response_usage, blocked_responses_api_usage,
    blocked_responses_stream_usage,
};
use crate::guardrail_message_scope::{Object, StreamItem};

fn json(source: &str) -> GuardrailValue {
    serde_json::from_str(source).unwrap()
}

fn response(source: &str) -> OriginalResponse {
    let GuardrailValue::Object(fields) = json(source) else {
        panic!("not an object: {source}");
    };
    OriginalResponse::Response(fields)
}

fn stream(items: &[&str]) -> OriginalResponse {
    OriginalResponse::Stream(
        items
            .iter()
            .map(|item| StreamItem::Json(json(item)))
            .collect(),
    )
}

fn usage(input_tokens: i64, output_tokens: i64) -> BlockedUsage {
    BlockedUsage {
        input_tokens,
        output_tokens,
    }
}

fn responses_usage(input_tokens: i64, output_tokens: i64, total_tokens: i64) -> ResponsesUsage {
    ResponsesUsage {
        input_tokens,
        output_tokens,
        total_tokens,
        details: Object::new(),
    }
}

#[test]
fn a_pre_call_block_reports_zero_usage() {
    assert_eq!(
        blocked_response_usage(&OriginalResponse::Absent),
        usage(0, 0)
    );
    assert_eq!(
        blocked_responses_api_usage(&OriginalResponse::Absent),
        responses_usage(0, 0, 0)
    );
    assert_eq!(blocked_chat_stream_usage(&OriginalResponse::Absent), (0, 0));
    assert_eq!(
        blocked_responses_stream_usage(&OriginalResponse::Absent),
        responses_usage(0, 0, 0)
    );
}

#[rstest]
#[case::anthropic_names(r#"{"usage":{"input_tokens":5,"output_tokens":7}}"#, usage(5, 7))]
#[case::chat_names(
    r#"{"usage":{"prompt_tokens":11,"completion_tokens":13}}"#,
    usage(11, 13)
)]
#[case::input_name_wins_over_prompt_name(
    r#"{"usage":{"input_tokens":1,"prompt_tokens":9,"output_tokens":2,"completion_tokens":9}}"#,
    usage(1, 2)
)]
#[case::null_input_name_does_not_fall_back(r#"{"usage":{"input_tokens":null,"prompt_tokens":9,"output_tokens":null,"completion_tokens":9}}"#, usage(0, 0))]
#[case::zero_input_name_does_not_fall_back(
    r#"{"usage":{"input_tokens":0,"prompt_tokens":9}}"#,
    usage(0, 0)
)]
#[case::one_side_missing(r#"{"usage":{"prompt_tokens":4}}"#, usage(4, 0))]
#[case::no_usage_key(r#"{"id":"x"}"#, usage(0, 0))]
#[case::null_usage(r#"{"usage":null}"#, usage(0, 0))]
#[case::usage_is_not_an_object(r#"{"usage":5}"#, usage(0, 0))]
#[case::token_is_not_a_number(r#"{"usage":{"input_tokens":"5","output_tokens":[]}}"#, usage(0, 0))]
#[case::huge_token_count_saturates(
    r#"{"usage":{"input_tokens":18446744073709551615}}"#,
    usage(i64::MAX, 0)
)]
#[case::negative_count_is_kept(r#"{"usage":{"input_tokens":-3}}"#, usage(-3, 0))]
fn a_response_object_reports_its_usage(#[case] source: &str, #[case] expected: BlockedUsage) {
    assert_eq!(blocked_response_usage(&response(source)), expected);
}

#[rstest]
#[case::input_and_output_from_message_start_and_delta(
    &[r#"{"type":"message_start","message":{"usage":{"input_tokens":12,"output_tokens":1}}}"#,
      r#"{"type":"message_delta","usage":{"output_tokens":7}}"#],
    Some(usage(12, 7))
)]
#[case::later_events_overwrite_earlier(
    &[r#"{"type":"message_start","message":{"usage":{"input_tokens":12,"output_tokens":1}}}"#,
      r#"{"type":"message_delta","usage":{"input_tokens":20,"output_tokens":9}}"#],
    Some(usage(20, 9))
)]
#[case::only_output_found(&[r#"{"type":"message_delta","usage":{"output_tokens":3}}"#], Some(usage(0, 3)))]
#[case::null_count_is_not_found(&[r#"{"type":"message_delta","usage":{"output_tokens":null}}"#], None)]
#[case::zero_count_is_found(&[r#"{"type":"message_delta","usage":{"output_tokens":0}}"#], Some(usage(0, 0)))]
#[case::other_event_types_are_ignored(
    &[r#"{"type":"content_block_delta","usage":{"output_tokens":3}}"#,
      r#"{"type":"message_stop"}"#,
      r#"{"usage":{"output_tokens":3}}"#],
    None
)]
#[case::message_start_reads_the_message_usage_only(&[r#"{"type":"message_start","usage":{"input_tokens":4}}"#], None)]
#[case::message_delta_reads_the_event_usage_only(&[r#"{"type":"message_delta","message":{"usage":{"input_tokens":4}}}"#], None)]
#[case::message_without_usage(&[r#"{"type":"message_start","message":{}}"#, r#"{"type":"message_start"}"#], None)]
#[case::usage_that_is_not_an_object(&[r#"{"type":"message_delta","usage":7}"#], None)]
#[case::sse_text_items(
    &[r#""event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":2}}}\n\n""#,
      r#""data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":6}}\n\ndata: [DONE]\n""#],
    Some(usage(2, 6))
)]
#[case::empty_stream(&[], None)]
fn anthropic_stream_chunks_carry_usage(
    #[case] sources: &[&str],
    #[case] expected: Option<BlockedUsage>,
) {
    let items: Vec<StreamItem> = sources
        .iter()
        .map(|item| StreamItem::Json(json(item)))
        .collect();
    assert_eq!(anthropic_stream_usage(&items), expected);
}

#[test]
fn anthropic_stream_bytes_carry_usage() {
    let items = [StreamItem::Bytes(
        b"data: {\"type\":\"message_delta\",\"usage\":{\"input_tokens\":3,\"output_tokens\":4}}\n\n"
            .to_vec(),
    )];
    assert_eq!(anthropic_stream_usage(&items), Some(usage(3, 4)));
}

#[test]
fn a_stream_with_usage_wins_over_the_response_shape() {
    let original =
        stream(&[r#"{"type":"message_delta","usage":{"input_tokens":3,"output_tokens":4}}"#]);
    assert_eq!(blocked_response_usage(&original), usage(3, 4));
}

#[test]
fn a_stream_without_usage_reports_zero() {
    let original = stream(&[r#"{"type":"message_stop"}"#, r#""data: [DONE]""#]);
    assert_eq!(blocked_response_usage(&original), usage(0, 0));
}

#[rstest]
#[case::native_names(
    r#"{"usage":{"input_tokens":5,"output_tokens":7,"total_tokens":20}}"#,
    responses_usage(5, 7, 20)
)]
#[case::total_is_the_sum_when_absent(
    r#"{"usage":{"input_tokens":5,"output_tokens":7}}"#,
    responses_usage(5, 7, 12)
)]
#[case::total_is_the_sum_when_zero(
    r#"{"usage":{"input_tokens":5,"output_tokens":7,"total_tokens":0}}"#,
    responses_usage(5, 7, 12)
)]
#[case::chat_names_are_mapped(
    r#"{"usage":{"prompt_tokens":2,"completion_tokens":3,"total_tokens":9}}"#,
    responses_usage(2, 3, 9)
)]
#[case::no_usage("{}", responses_usage(0, 0, 0))]
fn a_response_object_reports_responses_usage(
    #[case] source: &str,
    #[case] expected: ResponsesUsage,
) {
    assert_eq!(blocked_responses_api_usage(&response(source)), expected);
}

#[rstest]
#[case::details_and_cost_pass_through(
    r#"{"usage":{"input_tokens":5,"output_tokens":7,"total_tokens":12,"input_tokens_details":{"cached_tokens":2},"cost":0.5}}"#,
    &["cost", "input_tokens_details"]
)]
#[case::null_fields_are_dropped(
    r#"{"usage":{"input_tokens":5,"output_tokens":7,"total_tokens":12,"output_tokens_details":null,"cost":1}}"#,
    &["cost"]
)]
#[case::huge_counts_still_pass_through(
    r#"{"usage":{"input_tokens":18446744073709551615,"output_tokens":18446744073709551615,"total_tokens":18446744073709551615,"cost":1}}"#,
    &["cost"]
)]
#[case::no_total_means_a_rebuilt_usage(
    r#"{"usage":{"input_tokens":5,"output_tokens":7,"cost":0.5}}"#,
    &[]
)]
#[case::no_output_means_a_rebuilt_usage(
    r#"{"usage":{"input_tokens":5,"total_tokens":12,"cost":0.5}}"#,
    &[]
)]
#[case::no_input_means_a_rebuilt_usage(
    r#"{"usage":{"prompt_tokens":5,"output_tokens":7,"total_tokens":12,"cost":0.5}}"#,
    &[]
)]
#[case::chat_usage_is_rebuilt(
    r#"{"usage":{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5,"cost":0.5}}"#,
    &[]
)]
#[case::a_text_count_is_not_a_typed_usage(
    r#"{"usage":{"input_tokens":5,"output_tokens":7,"total_tokens":"12","cost":0.5}}"#,
    &[]
)]
fn a_typed_responses_usage_keeps_its_other_fields(#[case] source: &str, #[case] kept: &[&str]) {
    let usage = blocked_responses_api_usage(&response(source));
    assert_eq!(usage.details.keys().collect::<Vec<_>>(), kept);
}

#[test]
fn the_kept_fields_keep_their_values() {
    let usage = blocked_responses_api_usage(&response(
        r#"{"usage":{"input_tokens":5,"output_tokens":7,"total_tokens":12,"input_tokens_details":{"cached_tokens":2}}}"#,
    ));
    assert_eq!(
        serde_json::to_string(&usage).unwrap(),
        r#"{"input_tokens":5,"output_tokens":7,"total_tokens":12,"input_tokens_details":{"cached_tokens":2}}"#
    );
}

#[rstest]
#[case::last_chunk_with_usage(
    &[r#"{"usage":{"prompt_tokens":1,"completion_tokens":1}}"#,
      r#"{"usage":{"prompt_tokens":6,"completion_tokens":8}}"#,
      r#"{"choices":[]}"#],
    (6, 8)
)]
#[case::input_names_are_the_fallback(&[r#"{"usage":{"input_tokens":6,"output_tokens":8}}"#], (6, 8))]
#[case::prompt_name_wins_over_input_name(&[r#"{"usage":{"prompt_tokens":1,"input_tokens":9,"completion_tokens":2,"output_tokens":9}}"#], (1, 2))]
#[case::null_usage_is_skipped(&[r#"{"usage":{"prompt_tokens":6,"completion_tokens":8}}"#, r#"{"usage":null}"#], (6, 8))]
#[case::no_final_usage(&[r#"{"choices":[]}"#, r#"{"choices":[]}"#], (0, 0))]
#[case::usage_that_is_not_an_object(&[r#"{"usage":3}"#], (0, 0))]
#[case::empty_stream(&[], (0, 0))]
fn a_chat_stream_reports_the_last_chunk_usage(
    #[case] items: &[&str],
    #[case] expected: (i64, i64),
) {
    assert_eq!(blocked_chat_stream_usage(&stream(items)), expected);
}

#[test]
fn a_chat_stream_skips_items_without_fields() {
    let original = OriginalResponse::Stream(vec![
        StreamItem::Json(json(
            r#"{"usage":{"prompt_tokens":6,"completion_tokens":8}}"#,
        )),
        StreamItem::Json(GuardrailValue::Text("data: {}".to_owned())),
        StreamItem::Bytes(b"x".to_vec()),
    ]);
    assert_eq!(blocked_chat_stream_usage(&original), (6, 8));
}

#[test]
fn a_chat_response_object_defers_to_the_response_usage() {
    let original = response(r#"{"usage":{"prompt_tokens":6,"completion_tokens":8}}"#);
    assert_eq!(blocked_chat_stream_usage(&original), (6, 8));
}

#[rstest]
#[case::completed_event(
    &[r#"{"type":"response.created","response":{"usage":{"input_tokens":1,"output_tokens":1}}}"#,
      r#"{"type":"response.completed","response":{"usage":{"input_tokens":5,"output_tokens":7,"total_tokens":15}}}"#,
      r#"{"type":"response.output_text.delta"}"#],
    responses_usage(5, 7, 15)
)]
#[case::last_completed_event_wins(
    &[r#"{"type":"response.completed","response":{"usage":{"input_tokens":1,"output_tokens":1}}}"#,
      r#"{"type":"response.completed","response":{"usage":{"input_tokens":5,"output_tokens":7}}}"#],
    responses_usage(5, 7, 12)
)]
#[case::completed_without_response_is_skipped(
    &[r#"{"type":"response.completed","response":{"usage":{"input_tokens":5,"output_tokens":7}}}"#,
      r#"{"type":"response.completed","response":null}"#,
      r#"{"type":"response.completed"}"#],
    responses_usage(5, 7, 12)
)]
#[case::other_event_types_are_ignored(&[r#"{"type":"response.created","response":{"usage":{"input_tokens":5}}}"#], responses_usage(0, 0, 0))]
#[case::no_completed_event(&[r#"{"type":"response.output_text.delta"}"#], responses_usage(0, 0, 0))]
#[case::completed_response_without_usage(&[r#"{"type":"response.completed","response":{}}"#], responses_usage(0, 0, 0))]
fn a_responses_stream_reports_the_completed_event_usage(
    #[case] items: &[&str],
    #[case] expected: ResponsesUsage,
) {
    assert_eq!(blocked_responses_stream_usage(&stream(items)), expected);
}

#[test]
fn a_responses_object_defers_to_the_api_usage() {
    let original = response(r#"{"usage":{"input_tokens":5,"output_tokens":7}}"#);
    assert_eq!(
        blocked_responses_stream_usage(&original),
        responses_usage(5, 7, 12)
    );
}

#[test]
fn a_response_that_is_an_empty_object_reports_zero() {
    assert_eq!(
        blocked_response_usage(&OriginalResponse::Response(Object::new())),
        usage(0, 0)
    );
}
