#![cfg(test)]

use betterlite_guardrails::guardrail_message_scope::blocked_usage::OriginalResponse;
use rstest::rstest;
use uuid::Uuid;

use crate::guardrails::chat_guardrail_translation::block_framing::block_sse_chunks::{
    BlockedResponse, build_block_sse_chunks,
};
use crate::guardrails::chat_guardrail_translation::block_framing::blocked_identity::FreshIdentity;
use crate::guardrails::chat_guardrail_translation::extraction::fixtures::{
    frame_json, json, stream_items,
};

const FRESH: FreshIdentity = FreshIdentity {
    id: Uuid::from_u128(0xabc),
    created: 42,
};

fn blocked() -> BlockedResponse {
    BlockedResponse {
        message: "Blocked by policy".to_owned(),
        model: "gpt-x".to_owned(),
        original_response: OriginalResponse::Absent,
    }
}

#[rstest]
#[case::before_the_stream_starts(
    false,
    r#"{"id": "chatcmpl-00000000-0000-0000-0000-000000000abc", "object": "chat.completion.chunk", "created": 42, "model": "gpt-x", "choices": [{"index": 0, "delta": {"role": "assistant", "content": "Blocked by policy"}, "finish_reason": null}]}"#,
)]
#[case::while_the_stream_runs(
    true,
    r#"{"id": "chatcmpl-00000000-0000-0000-0000-000000000abc", "object": "chat.completion.chunk", "created": 42, "model": "gpt-x", "choices": [{"index": 0, "delta": {"content": "Blocked by policy"}, "finish_reason": null}]}"#,
)]
fn only_a_block_before_the_stream_opens_the_completion_with_a_role(
    #[case] stream_started: bool,
    #[case] expected: &str,
) {
    let frames = build_block_sse_chunks(&blocked(), stream_started, &[], &FRESH).unwrap();
    assert_eq!(frame_json(&frames[0]), json(expected));
}

#[rstest]
#[case::before_the_stream_starts(false)]
#[case::while_the_stream_runs(true)]
fn the_final_chunk_has_an_empty_delta_and_a_content_filter_finish(#[case] stream_started: bool) {
    let frames = build_block_sse_chunks(&blocked(), stream_started, &[], &FRESH).unwrap();
    let expected = r#"{"id": "chatcmpl-00000000-0000-0000-0000-000000000abc", "object": "chat.completion.chunk", "created": 42, "model": "gpt-x", "choices": [{"index": 0, "delta": {}, "finish_reason": "content_filter"}], "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}}"#;
    assert_eq!(frame_json(&frames[1]), json(expected));
}

#[rstest]
#[case::message_chunk(0)]
#[case::final_chunk(1)]
fn both_chunks_reuse_the_identity_of_the_stream_so_far(#[case] position: usize) {
    let seen = stream_items(vec![json(
        r#"{"id": "chatcmpl-seen", "created": 5, "model": "seen-model"}"#,
    )]);
    let frames = build_block_sse_chunks(&blocked(), true, &seen, &FRESH).unwrap();
    let chunk = frame_json(&frames[position]);
    assert_eq!(chunk["id"], "chatcmpl-seen");
    assert_eq!(chunk["created"], json("5"));
    assert_eq!(chunk["model"], "seen-model");
}

#[rstest]
#[case::message_chunk(0)]
#[case::final_chunk(1)]
fn each_chunk_is_a_complete_data_frame(#[case] position: usize) {
    let frames = build_block_sse_chunks(&blocked(), false, &[], &FRESH).unwrap();
    assert!(frames[position].starts_with(b"data: {"));
    assert!(frames[position].ends_with(b"}\n\n"));
}
