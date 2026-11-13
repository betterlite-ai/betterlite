#![cfg(test)]

use std::str;

use betterlite_config::sections::mock_response::UsageDefaults;
use betterlite_openai::chat::ChatRequest as ClientChatRequest;
use betterlite_openai::stream::chunk::Chunk;
use rstest::rstest;

use super::mock_frames;
use crate::mock::stream::StreamSpec;

const PLAIN: &[u8] = br#"{"model":"g","messages":[]}"#;
const WITH_USAGE: &[u8] =
    br#"{"model":"g","messages":[],"stream":true,"stream_options":{"include_usage":true}}"#;
const DONE: &[u8] = b"data: [DONE]\n\n";

// every frame before the closing [DONE], decoded as the chunk it carries
fn mock_stream_chunks(
    body: &[u8],
    prompt_tokens: Option<u64>,
    choice_count: Option<u32>,
) -> Vec<Chunk> {
    let request = ClientChatRequest::parse(body).expect("mock request");
    let spec = StreamSpec {
        content: "abcdefg",
        choice_count,
        model: "g",
        id: "chatcmpl-1",
        created: 7,
    };
    let usage = UsageDefaults {
        prompt_tokens: 10,
        completion_tokens: 20,
    };
    let mut frames = mock_frames(&spec, &request, prompt_tokens, usage).expect("frames");
    assert_eq!(frames.pop().as_deref(), Some(DONE));
    frames
        .iter()
        .map(|frame| {
            let text = str::from_utf8(frame).expect("utf-8 frame");
            let json = text
                .strip_prefix("data: ")
                .and_then(|rest| rest.strip_suffix("\n\n"))
                .expect("sse frame");
            serde_json::from_str(json).expect("chunk")
        })
        .collect()
}

#[test]
fn mock_frames_slice_the_content_and_keep_the_role_on_the_first_delta() {
    let chunks = mock_stream_chunks(PLAIN, None, None);
    assert_eq!(chunks.len(), 4);
    let deltas: Vec<_> = chunks[..3]
        .iter()
        .map(|chunk| &chunk.choices[0].delta)
        .collect();
    assert_eq!(deltas[0].role.as_deref(), Some("assistant"));
    assert_eq!(deltas[0].content.as_deref(), Some("abc"));
    assert_eq!(deltas[1].role, None);
    assert_eq!(deltas[1].content.as_deref(), Some("def"));
    assert_eq!(deltas[2].role, None);
    assert_eq!(deltas[2].content.as_deref(), Some("g"));
    assert_eq!(chunks[3].choices[0].finish_reason.as_deref(), Some("stop"));
    assert_eq!(chunks[3].id, "chatcmpl-1");
    assert_eq!(chunks[3].created, 7);
}

#[rstest]
#[case::asked_for(WITH_USAGE, None, Some((10, 30)))]
#[case::prompt_count_from_the_plan(PLAIN, Some(7), Some((7, 27)))]
#[case::not_asked_for(PLAIN, None, None)]
fn mock_frames_end_with_a_usage_chunk_only_when_asked_for(
    #[case] body: &[u8],
    #[case] prompt_tokens: Option<u64>,
    #[case] expected: Option<(u64, u64)>,
) {
    let chunks = mock_stream_chunks(body, prompt_tokens, None);
    let usage = chunks[3..].last().and_then(|chunk| chunk.usage.as_ref());
    assert_eq!(
        usage.map(|counts| (counts.prompt_tokens, counts.total_tokens)),
        expected
    );
    assert_eq!(chunks.len(), expected.map_or(4, |_| 5));
    assert!(chunks[..4].iter().all(|chunk| chunk.usage.is_none()));
}

#[test]
fn mock_frames_drop_the_content_chunks_when_no_choice_is_asked_for() {
    let chunks = mock_stream_chunks(PLAIN, None, Some(0));
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].choices[0].finish_reason.as_deref(), Some("stop"));
}
