#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use betterlite_guardrails::guardrail_message_scope::blocked_usage::OriginalResponse;
use rstest::rstest;
use uuid::Uuid;

use crate::guardrails::chat_guardrail_translation::block_framing::block_sse_chunks::{
    BlockedResponse, build_block_sse_chunks,
};
use crate::guardrails::chat_guardrail_translation::block_framing::blocked_identity::FreshIdentity;
use crate::guardrails::chat_guardrail_translation::extraction::fixtures::{
    frame_json, json, object, stream_items,
};

const FRESH: FreshIdentity = FreshIdentity {
    id: Uuid::from_u128(1),
    created: 1,
};

fn final_usage(original_response: OriginalResponse) -> JsonValue {
    let exception = BlockedResponse {
        message: "no".to_owned(),
        model: "m".to_owned(),
        original_response,
    };
    let frames = build_block_sse_chunks(&exception, false, &[], &FRESH).unwrap();
    frame_json(&frames[1])["usage"].clone()
}

fn usage(prompt: i64, completion: i64, total: i64) -> JsonValue {
    json(&format!(
        r#"{{"prompt_tokens": {prompt}, "completion_tokens": {completion}, "total_tokens": {total}}}"#
    ))
}

fn stream(chunks: Vec<JsonValue>) -> OriginalResponse {
    OriginalResponse::Stream(stream_items(chunks))
}

#[rstest]
#[case::absent(OriginalResponse::Absent, usage(0, 0, 0))]
#[case::response_prompt_and_completion_tokens(
    OriginalResponse::Response(object(json(
        r#"{"usage": {"prompt_tokens": 3, "completion_tokens": 4}}"#
    ))),
    usage(3, 4, 7)
)]
#[case::response_input_and_output_tokens(
    OriginalResponse::Response(object(json(
        r#"{"usage": {"input_tokens": 5, "output_tokens": 6}}"#
    ))),
    usage(5, 6, 11)
)]
#[case::response_without_usage(
    OriginalResponse::Response(object(json(r#"{"id": "r"}"#))),
    usage(0, 0, 0)
)]
#[case::stream_takes_the_last_usage(
    stream(vec![
        json(r#"{"usage": {"prompt_tokens": 1, "completion_tokens": 1}}"#),
        json(r#"{"choices": []}"#),
        json(r#"{"usage": {"prompt_tokens": 8, "completion_tokens": 9}}"#),
        json(r#"{"usage": null}"#),
    ]),
    usage(8, 9, 17),
)]
#[case::stream_input_and_output_names(
    stream(vec![json(r#"{"usage": {"input_tokens": 2, "output_tokens": 3}}"#)]),
    usage(2, 3, 5),
)]
#[case::stream_without_usage(stream(vec![json(r#"{"choices": []}"#)]), usage(0, 0, 0))]
#[case::empty_stream(stream(vec![]), usage(0, 0, 0))]
fn the_final_chunk_carries_the_blocked_usage(
    #[case] original_response: OriginalResponse,
    #[case] expected: JsonValue,
) {
    assert_eq!(final_usage(original_response), expected);
}
