#![cfg(test)]

use super::{CallType, get_call_types_for_route, pattern::parse_route};
use std::{fs, path::Path, str};

const CHAT: &[CallType] = &[CallType::Acompletion, CallType::Completion];
const RESPONSE: &[CallType] = &[CallType::Aresponses, CallType::Responses];
const GEN: &[CallType] = &[CallType::AgenerateContent, CallType::GenerateContent];
const STREAM: &[CallType] = &[
    CallType::AgenerateContentStream,
    CallType::GenerateContentStream,
];
const ANTHROPIC: &[CallType] = &[CallType::AnthropicMessages];

#[rstest::rstest]
#[case::chat("/chat/completions", Some(CHAT))]
#[case::chat_versioned("/v1/chat/completions", Some(CHAT))]
#[case::chat_engine("/engines/gpt/chat/completions", Some(CHAT))]
#[case::chat_deployment("/openai/deployments/deploy/chat/completions", Some(CHAT))]
#[case::responses("/responses", Some(RESPONSE))]
#[case::responses_versioned("/v1/responses", Some(RESPONSE))]
#[case::responses_openai("/openai/v1/responses", Some(RESPONSE))]
#[case::response_id("/responses/resp", Some(RESPONSE))]
#[case::response_id_versioned("/v1/responses/resp", Some(RESPONSE))]
#[case::response_id_openai("/openai/v1/responses/resp", Some(RESPONSE))]
#[case::anthropic("/anthropic/v1/messages", Some(ANTHROPIC))]
#[case::messages("/v1/messages", Some(ANTHROPIC))]
#[case::generate("/generate_content", Some(GEN))]
#[case::generate_model("/models/model:generateContent", Some(GEN))]
#[case::generate_beta("/v1beta/models/model:generateContent", Some(GEN))]
#[case::mcp("/mcp/call_tool", Some(&[CallType::CallMcpTool][..]))]
#[case::stream("/generate_content_stream", Some(STREAM))]
#[case::stream_model("/models/model:streamGenerateContent", Some(STREAM))]
#[case::stream_beta("/v1beta/models/org/model:streamGenerateContent", Some(STREAM))]
#[case::multi_segment_tail("/models/org/model:generateContent", Some(GEN))]
#[case::suffix_slash_value("/models//:generateContent", Some(GEN))]
#[case::empty_suffix("/models/:generateContent", None)]
#[case::suffix_boundary("/models/model:generateContentExtra", None)]
#[case::suffix_wrong_case("/models/model:generatecontent", None)]
#[case::suffix_extra_segment("/models/model:generateContent/extra", None)]
#[case::empty_placeholder("/engines//chat/completions", Some(CHAT))]
#[case::bare_no_multisegment("/engines/org/model/chat/completions", None)]
#[case::trailing_slashes("///v1/chat/completions///", Some(CHAT))]
#[case::missing_leading_slash("v1/chat/completions", Some(CHAT))]
#[case::unknown("/no-such-route", None)]
#[case::empty("", None)]
#[case::slashes("///", None)]
#[case::short_route("/engines/gpt/chat", None)]
#[case::long_route("/v1/chat/completions/extra", None)]
#[case::wrong_literal("/engines/gpt/wrong/completions", None)]
#[case::query_not_stripped("/v1/chat/completions?x=1", None)]
#[case::exact_precedence("/videos/characters", Some(&[CallType::AvideoCreateCharacter, CallType::VideoCreateCharacter][..]))]
#[case::pattern_order("/videos/characters/", Some(&[CallType::AvideoRetrieve, CallType::VideoRetrieve, CallType::AvideoDelete, CallType::VideoDelete][..]))]
#[case::single_placeholder("/v1/threads/thread-1", Some(&[CallType::AgetThread, CallType::GetThread][..]))]
#[case::batch("/v1/batches/batch-1", Some(&[CallType::AretrieveBatch, CallType::RetrieveBatch][..]))]
fn call_route_metadata_route_metadata_tests_behavior(
    #[case] route: &str,
    #[case] expected: Option<&[CallType]>,
) {
    assert_eq!(
        get_call_types_for_route(route).unwrap().as_deref(),
        expected
    );
}

#[rstest::rstest]
#[case::suffixed_multi_tail("suffixed-multi-tail")]
fn replays_the_route_pattern_corpus(#[case] seed: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz/corpus/route_pattern")
        .join(seed);
    let bytes = fs::read(path).unwrap();
    let route = str::from_utf8(&bytes).unwrap();
    drop(parse_route(route));
    drop(get_call_types_for_route(route));
}
