#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;
use uuid::Uuid;

use crate::gemini_candidates::blocked::{FLAGGED_FINISH_REASONS, complete_block, stream_block};
use crate::gemini_candidates::candidate::{Candidate, PromptFeedback};
use crate::gemini_candidates::error::CandidatesError;
use crate::gemini_candidates::process::{Mode, Processed, process_candidates};
use crate::gemini_response_parts::projection::{CallMode, PartContext};
use crate::gemini_usage::error::UsageError;
use crate::gemini_usage::metadata::UsageMetadata;

const NOW: i64 = 1_700_000_000;
const HEX: &str = "0123456789abcdef0123456789abcdef";
const AUDIO_ID: &str = "01234567-89ab-cdef-0123-456789abcdef";
const NULL_MESSAGE: &str = r#"{"role":"assistant","content":null,"reasoning_content":null,"thinking_blocks":null,"tool_calls":null,"function_call":null,"images":null,"audio":null,"annotations":null,"provider_specific_fields":null}"#;

fn candidates(source: &str) -> Vec<Candidate> {
    serde_json::from_str(source).unwrap()
}

fn expected(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn rendered<T>(value: &T) -> Json
where
    T: serde::Serialize,
{
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}

// walks object keys and list positions; a step that leads nowhere reads as null
fn at(value: &Json, path: &str) -> Json {
    let mut current = value;
    for step in path.split('.') {
        let next = match current {
            Json::Object(fields) => fields.get(step),
            Json::List(items) => step
                .parse::<usize>()
                .ok()
                .and_then(|position| items.get(position)),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => None,
        };
        let Some(found) = next else {
            return Json::Null;
        };
        current = found;
    }
    current.clone()
}

fn context(cumulative_tool_call_index: u32, call_mode: CallMode) -> PartContext {
    PartContext {
        cumulative_tool_call_index,
        call_mode,
        now: NOW,
    }
}

fn run_with(source: &str, mode: Mode, context: PartContext) -> Processed {
    let mut ids = || Uuid::parse_str(HEX).unwrap();
    process_candidates(&candidates(source), mode, context, &mut ids).unwrap()
}

fn run(source: &str, mode: Mode) -> Processed {
    run_with(source, mode, context(0, CallMode::Tools))
}

fn pick(source: &str, mode: Mode, path: &str) -> Json {
    at(&rendered(&run(source, mode).choices), path)
}

fn usage(prompt: i64, completion: i64, total: i64) -> UsageMetadata {
    serde_json::from_str(&format!(
        r#"{{"promptTokenCount":{prompt},"candidatesTokenCount":{completion},"totalTokenCount":{total}}}"#
    ))
    .unwrap()
}

fn feedback(source: &str) -> PromptFeedback {
    serde_json::from_str(source).unwrap()
}

fn with_parts(parts: &str) -> String {
    format!(r#"[{{"content":{{"parts":{parts}}},"finishReason":"STOP"}}]"#)
}

#[rstest]
#[case::empty_candidate("[{}]", "[]")]
#[case::null_content_only(r#"[{"content":null}]"#, "[]")]
#[case::content_only(r#"[{"content":{"parts":[{"text":"a"}]}}]"#, "[0]")]
#[case::content_without_parts(r#"[{"content":{}}]"#, "[0]")]
#[case::finish_reason_only(r#"[{"finishReason":"STOP"}]"#, "[0]")]
#[case::skipped_candidates_count_in_the_position(r#"[{},{"finishReason":"STOP"}]"#, "[1]")]
#[case::explicit_index_wins(
    r#"[{"index":5,"finishReason":"STOP"},{"finishReason":"STOP"}]"#,
    "[5,1]"
)]
#[case::explicit_zero_index_wins(r#"[{},{"index":0,"finishReason":"STOP"}]"#, "[0]")]
fn candidate_indexes_follow_the_skip_rule(#[case] source: &str, #[case] indexes: &str) {
    for (mode, field) in [(Mode::Complete, "message"), (Mode::Streaming, "delta")] {
        let Json::List(found) = rendered(&run(source, mode).choices) else {
            panic!("choices are a list");
        };
        let positions: Vec<Json> = found.iter().map(|choice| at(choice, "index")).collect();
        assert_eq!(Json::List(positions), expected(indexes));
        assert!(found.iter().all(|choice| at(choice, field) != Json::Null));
    }
}

#[test]
fn text_candidate_serializes_every_message_field() {
    let source = with_parts(r#"[{"text":"hi"}]"#);
    assert_eq!(
        rendered(&run(&source, Mode::Complete).choices),
        expected(
            r#"[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"hi","reasoning_content":null,"thinking_blocks":[],"tool_calls":null,"function_call":null,"images":[],"audio":null,"annotations":null,"provider_specific_fields":null},"logprobs":null,"native_finish_reason":"STOP"}]"#
        )
    );
}

#[test]
fn candidate_without_parts_leaves_the_list_fields_null() {
    let source = r#"[{"finishReason":"STOP"}]"#;
    assert_eq!(
        rendered(&run(source, Mode::Complete).choices),
        expected(&format!(
            r#"[{{"index":0,"finish_reason":"stop","message":{NULL_MESSAGE},"logprobs":null,"native_finish_reason":"STOP"}}]"#
        ))
    );
}

#[test]
fn candidate_without_finish_reason_has_no_native_reason() {
    let source = r#"[{"content":{"parts":[{"text":"hi"}]}}]"#;
    assert_eq!(
        pick(source, Mode::Complete, "0.finish_reason"),
        expected(r#""stop""#)
    );
    assert_eq!(
        pick(source, Mode::Complete, "0.native_finish_reason"),
        Json::Null
    );
}

#[rstest]
#[case::audio_only(r#"[{"inlineData":{"mimeType":"audio/wav","data":"QUJD"}}]"#, "null")]
#[case::audio_then_text(
    r#"[{"inlineData":{"mimeType":"audio/wav","data":"QUJD"}},{"text":"spoken words"}]"#,
    r#""spoken words""#
)]
#[case::text_then_audio(
    r#"[{"text":"spoken words"},{"inlineData":{"mimeType":"audio/wav","data":"QUJD"}}]"#,
    r#""spoken words""#
)]
fn audio_leaves_the_content_empty_until_text_arrives(#[case] parts: &str, #[case] content: &str) {
    let source = with_parts(parts);
    assert_eq!(
        pick(&source, Mode::Complete, "0.message.content"),
        expected(content)
    );
    assert_eq!(
        pick(&source, Mode::Complete, "0.message.audio"),
        expected(&format!(
            r#"{{"id":"{AUDIO_ID}","data":"QUJD","expires_at":1700086400,"transcript":""}}"#
        ))
    );
}

#[rstest]
#[case::function_call_beats_stop(
    r#"{"functionCall":{"name":"lookup","args":{}}}"#,
    "STOP",
    "tool_calls"
)]
#[case::function_call_beats_max_tokens(
    r#"{"functionCall":{"name":"lookup","args":{}}}"#,
    "MAX_TOKENS",
    "tool_calls"
)]
#[case::native_reason_is_mapped(r#"{"text":"plain"}"#, "MAX_TOKENS", "length")]
#[case::unmapped_reason_stops(r#"{"text":"plain"}"#, "SOMETHING_NEW", "stop")]
#[case::stop_reason_stops(r#"{"text":"plain"}"#, "STOP", "stop")]
fn finish_reason_prefers_calls_over_the_native_reason(
    #[case] part: &str,
    #[case] native: &str,
    #[case] finish: &str,
) {
    let source = format!(r#"[{{"content":{{"parts":[{part}]}},"finishReason":"{native}"}}]"#);
    assert_eq!(
        pick(&source, Mode::Complete, "0.finish_reason"),
        Json::Text(finish.to_owned())
    );
    assert_eq!(
        pick(&source, Mode::Complete, "0.native_finish_reason"),
        Json::Text(native.to_owned())
    );
}

#[test]
fn finish_reason_maps_the_native_reason_only_when_there_is_one() {
    let tools = context(0, CallMode::Tools);
    let with_native = run_with(
        r#"[{"content":{"parts":[{"text":"a"}]},"finishReason":"MAX_TOKENS"}]"#,
        Mode::Complete,
        tools,
    );
    let without_native = run_with(
        r#"[{"content":{"parts":[{"text":"a"}]}}]"#,
        Mode::Complete,
        tools,
    );
    assert_eq!(
        at(&rendered(&with_native.choices), "0.finish_reason"),
        expected(r#""length""#)
    );
    assert_eq!(
        at(&rendered(&without_native.choices), "0.finish_reason"),
        expected(r#""stop""#)
    );
}

#[test]
fn legacy_function_mode_reports_a_function_call() {
    let source = with_parts(r#"[{"functionCall":{"name":"lookup","args":{"a":1}}}]"#);
    let processed = run_with(
        &source,
        Mode::Complete,
        context(0, CallMode::LegacyFunction),
    );
    let found = rendered(&processed.choices);
    assert_eq!(
        at(&found, "0.finish_reason"),
        expected(r#""function_call""#)
    );
    assert_eq!(at(&found, "0.message.tool_calls"), Json::Null);
    assert_eq!(
        at(&found, "0.message.function_call"),
        expected(r#"{"name":"lookup","arguments":"{\"a\":1}"}"#)
    );
}

#[test]
fn tool_call_indexes_continue_across_candidates() {
    let source = r#"[
        {"content":{"parts":[{"functionCall":{"name":"lookup","args":{"city":"Paris"}}}]},"finishReason":"STOP"},
        {},
        {"content":{"parts":[{"functionCall":{"name":"forecast","args":{"days":3}}}]},"finishReason":"STOP"}
    ]"#;
    let processed = run_with(source, Mode::Complete, context(4, CallMode::Tools));
    assert_eq!(processed.next_tool_call_index, 6);
    let found = rendered(&processed.choices);
    assert_eq!(at(&found, "0.message.tool_calls.0.index"), expected("4"));
    assert_eq!(
        at(&found, "0.message.tool_calls.0.function"),
        expected(r#"{"name":"lookup","arguments":"{\"city\":\"Paris\"}"}"#)
    );
    assert_eq!(at(&found, "1.message.tool_calls.0.index"), expected("5"));
    assert_eq!(
        at(&found, "1.message.tool_calls.0.function.name"),
        expected(r#""forecast""#)
    );
}

#[test]
fn candidates_without_calls_keep_the_incoming_tool_call_index() {
    let source = with_parts(r#"[{"text":"a"}]"#);
    let processed = run_with(&source, Mode::Streaming, context(7, CallMode::Tools));
    assert_eq!(processed.next_tool_call_index, 7);
}

#[rstest]
#[case::joined_with_newlines(
    r#"[{"text":"data:audio/wav;base64,QUJD","thought":true},{"text":"","thought":true},{"text":"data:audio/mp3;base64,REVG","thought":true}]"#,
    r#""data:audio/wav;base64,QUJD\ndata:audio/mp3;base64,REVG""#
)]
#[case::single_block(
    r#"[{"text":"data:audio/wav;base64,QUJD","thought":true}]"#,
    r#""data:audio/wav;base64,QUJD""#
)]
#[case::only_empty_blocks(r#"[{"text":"","thought":true}]"#, "null")]
#[case::no_thought_parts(r#"[{"text":"visible"}]"#, "null")]
#[case::reasoning_from_the_parts_wins(
    r#"[{"text":"plan","thought":true},{"text":"data:audio/wav;base64,QUJD","thought":true}]"#,
    r#""plan""#
)]
fn streamed_reasoning_falls_back_to_the_thinking_blocks(
    #[case] parts: &str,
    #[case] reasoning: &str,
) {
    assert_eq!(
        pick(
            &with_parts(parts),
            Mode::Streaming,
            "0.delta.reasoning_content"
        ),
        expected(reasoning)
    );
}

#[test]
fn complete_choices_never_rebuild_reasoning_from_the_thinking_blocks() {
    let source = with_parts(r#"[{"text":"data:audio/wav;base64,QUJD","thought":true}]"#);
    assert_eq!(
        pick(&source, Mode::Complete, "0.message.reasoning_content"),
        Json::Null
    );
    assert_eq!(
        pick(
            &source,
            Mode::Complete,
            "0.message.thinking_blocks.0.thinking"
        ),
        expected(r#""data:audio/wav;base64,QUJD""#)
    );
}

#[test]
fn streamed_delta_has_no_role_thinking_blocks_or_audio() {
    let source = with_parts(
        r#"[{"inlineData":{"mimeType":"audio/wav","data":"QUJD"}},
            {"text":"hi"},
            {"text":"plan","thought":true,"thoughtSignature":"sig"}]"#,
    );
    assert_eq!(
        rendered(&run(&source, Mode::Streaming).choices),
        expected(
            r#"[{"index":0,"finish_reason":"stop","delta":{"role":null,"content":"hi","reasoning_content":"plan","tool_calls":null,"function_call":null,"images":null,"annotations":null,"provider_specific_fields":{"thought_signatures":["sig"]}},"logprobs":null}]"#
        )
    );
}

#[test]
fn images_are_kept_in_the_delta_only_when_there_are_some() {
    let source = with_parts(r#"[{"inlineData":{"mimeType":"image/png","data":"aW1n"}}]"#);
    let image = r#"[{"image_url":{"url":"data:image/png;base64,aW1n","detail":"auto"},"index":0,"type":"image_url"}]"#;
    assert_eq!(
        pick(&source, Mode::Streaming, "0.delta.images"),
        expected(image)
    );
    assert_eq!(
        pick(&source, Mode::Complete, "0.message.images"),
        expected(image)
    );
    let plain = with_parts(r#"[{"text":"hi"}]"#);
    assert_eq!(pick(&plain, Mode::Streaming, "0.delta.images"), Json::Null);
    assert_eq!(
        pick(&plain, Mode::Complete, "0.message.images"),
        expected("[]")
    );
}

const GROUNDED: &str = r#"{"groundingChunks":[{"web":{"uri":"https://example.com","title":"Example"}}],"groundingSupports":[{"segment":{"startIndex":0,"endIndex":4},"groundingChunkIndices":[0]}]}"#;
const CITATION: &str = r#"[{"type":"url_citation","url_citation":{"start_index":0,"end_index":4,"url":"https://example.com","title":"Example"}}]"#;

#[rstest]
#[case::with_parts(r#""content":{"parts":[{"text":"hi"}]},"#, CITATION)]
#[case::content_without_parts(r#""content":{},"#, "null")]
#[case::no_content("", "null")]
fn annotations_need_parts_and_a_matching_support(#[case] content: &str, #[case] found: &str) {
    let source = format!(r#"[{{{content}"finishReason":"STOP","groundingMetadata":{GROUNDED}}}]"#);
    assert_eq!(
        pick(&source, Mode::Complete, "0.message.annotations"),
        expected(found)
    );
    assert_eq!(
        pick(&source, Mode::Streaming, "0.delta.annotations"),
        expected(found)
    );
}

#[test]
fn grounding_without_supports_adds_no_annotations() {
    let source = r#"[{"content":{"parts":[{"text":"hi"}]},"finishReason":"STOP","groundingMetadata":{"groundingChunks":[]}}]"#;
    assert_eq!(
        pick(source, Mode::Complete, "0.message.annotations"),
        Json::Null
    );
}

#[rstest]
#[case::nothing(r#"[{"text":"hi"}]"#, "null", "null")]
#[case::signature(r#"[{"text":"hi","thoughtSignature":"sig"}]"#, r#"["sig"]"#, "null")]
#[case::server_tool(
    r#"[{"toolCall":{"toolType":"search","id":"one","args":{"q":"x"}}}]"#,
    "null",
    r#""one""#
)]
fn provider_specific_fields_carry_signatures_and_server_tools(
    #[case] parts: &str,
    #[case] signatures: &str,
    #[case] tool_id: &str,
) {
    let source = with_parts(parts);
    let fields = "0.message.provider_specific_fields";
    assert_eq!(
        pick(
            &source,
            Mode::Complete,
            &format!("{fields}.thought_signatures")
        ),
        expected(signatures)
    );
    assert_eq!(
        pick(
            &source,
            Mode::Complete,
            &format!("{fields}.server_side_tool_invocations.0.id")
        ),
        expected(tool_id)
    );
}

#[test]
fn logprobs_are_kept_for_a_candidate_without_parts() {
    let source = r#"[{"finishReason":"STOP","logprobsResult":{"chosenCandidates":[{"token":"a","logProbability":-0.5}]}}]"#;
    let token = expected(r#""a""#);
    assert_eq!(
        pick(source, Mode::Complete, "0.logprobs.content.0.token"),
        token
    );
    assert_eq!(
        pick(source, Mode::Streaming, "0.logprobs.content.0.token"),
        token
    );
    assert_eq!(
        pick(source, Mode::Complete, "0.message.content"),
        Json::Null
    );
}

#[test]
fn metadata_accumulates_over_the_kept_candidates() {
    let source = r#"[
        {"safetyRatings":[{"category":"skipped"}]},
        {"finishReason":"STOP","safetyRatings":[{"category":"first"}],"citationMetadata":{"citations":[]}},
        {"finishReason":"STOP","safetyRatings":[{"category":"second"}],"urlContextMetadata":{"urlMetadata":[]}}
    ]"#;
    let metadata = run(source, Mode::Complete).metadata;
    let ratings = expected(r#"[[{"category":"first"}],[{"category":"second"}]]"#);
    assert_eq!(rendered(&metadata.safety_ratings), ratings);
    assert_eq!(rendered(&metadata.safety_results), ratings);
    assert_eq!(
        rendered(&metadata.citation_metadata),
        expected(r#"[{"citations":[]}]"#)
    );
    assert_eq!(
        rendered(&metadata.url_context_metadata),
        expected(r#"[{"urlMetadata":[]}]"#)
    );
    assert!(metadata.grounding_metadata.is_empty());
}

#[test]
fn candidate_keeps_unknown_keys_as_metadata() {
    let parsed = candidates(
        r#"[{"content":null,"finishReason":"STOP","index":2,"safetyRatings":[],"avgLogprobs":-1.5}]"#,
    );
    let found = &parsed[0];
    assert!(found.content.is_none());
    assert_eq!(found.finish_reason.as_deref(), Some("STOP"));
    assert_eq!(found.index, Some(2));
    assert!(found.metadata.contains_key("safetyRatings"));
    assert!(found.metadata.contains_key("avgLogprobs"));
    assert!(!found.metadata.contains_key("finishReason"));
    assert!(!found.metadata.contains_key("index"));
}

#[rstest]
#[case::safety("SAFETY")]
#[case::recitation("RECITATION")]
#[case::blocklist("BLOCKLIST")]
#[case::prohibited_content("PROHIBITED_CONTENT")]
#[case::spii("SPII")]
#[case::image_safety("IMAGE_SAFETY")]
#[case::image_prohibited_content("IMAGE_PROHIBITED_CONTENT")]
fn a_flagged_first_candidate_blocks_the_response(#[case] reason: &str) {
    assert!(FLAGGED_FINISH_REASONS.contains(&reason));
    let source = format!(
        r#"[{{"content":{{"parts":[{{"text":"never shown"}}]}},"finishReason":"{reason}"}}]"#
    );
    let blocked = complete_block(None, &candidates(&source), Some(&usage(5, 2, 9)))
        .unwrap()
        .unwrap();
    assert_eq!(
        rendered(&blocked.choice),
        expected(&format!(
            r#"{{"index":0,"finish_reason":"content_filter","message":{NULL_MESSAGE},"logprobs":null,"native_finish_reason":null}}"#
        ))
    );
    assert_eq!(blocked.usage.prompt_tokens, 5);
    assert_eq!(blocked.usage.completion_tokens, 2);
    assert_eq!(blocked.usage.total_tokens, 9);
    assert!(blocked.usage.prompt_tokens_details.is_none());
    assert!(blocked.usage.completion_tokens_details.is_none());
}

#[rstest]
#[case::later_candidate(r#"[{"finishReason":"STOP"},{"finishReason":"SAFETY"}]"#)]
#[case::unflagged_first(r#"[{"finishReason":"MAX_TOKENS"}]"#)]
#[case::lowercase_reason(r#"[{"finishReason":"safety"}]"#)]
#[case::first_without_reason(r#"[{"content":{}},{"finishReason":"SAFETY"}]"#)]
#[case::no_candidates("[]")]
fn other_candidates_do_not_block_the_response(#[case] source: &str) {
    let found = complete_block(None, &candidates(source), Some(&usage(1, 1, 2))).unwrap();
    assert!(found.is_none());
}

#[test]
fn a_flagged_later_candidate_stays_an_ordinary_choice() {
    let source = r#"[
        {"content":{"parts":[{"text":"first"}]},"finishReason":"STOP"},
        {"content":{"parts":[{"text":"second"}]},"finishReason":"SAFETY"}
    ]"#;
    assert_eq!(
        pick(source, Mode::Complete, "1.finish_reason"),
        expected(r#""content_filter""#)
    );
    assert_eq!(
        pick(source, Mode::Complete, "1.native_finish_reason"),
        expected(r#""SAFETY""#)
    );
    assert_eq!(
        pick(source, Mode::Complete, "1.message.content"),
        expected(r#""second""#)
    );
}

#[rstest]
#[case::block_reason(r#"{"blockReason":"PROHIBITED_CONTENT"}"#, true)]
#[case::empty_reason_still_blocks(r#"{"blockReason":""}"#, true)]
#[case::null_reason(r#"{"blockReason":null}"#, false)]
#[case::no_reason("{}", false)]
fn a_block_reason_blocks_the_prompt(#[case] source: &str, #[case] blocked: bool) {
    let prompt = feedback(source);
    let complete = complete_block(Some(&prompt), &[], Some(&usage(7, 0, 7))).unwrap();
    assert_eq!(complete.is_some(), blocked);
    assert_eq!(stream_block(Some(&prompt)).is_some(), blocked);
}

#[test]
fn a_prompt_block_copies_the_raw_counters() {
    let prompt = feedback(r#"{"blockReason":"SAFETY"}"#);
    let blocked = complete_block(Some(&prompt), &[], Some(&usage(7, 0, 7)))
        .unwrap()
        .unwrap();
    assert_eq!(blocked.choice.finish_reason, "content_filter");
    assert_eq!(blocked.usage.prompt_tokens, 7);
    assert_eq!(blocked.usage.completion_tokens, 0);
    assert_eq!(blocked.usage.total_tokens, 7);
}

#[rstest]
#[case::by_prompt(r#"{"blockReason":"SAFETY"}"#, "[]")]
#[case::by_candidate("{}", r#"[{"finishReason":"SPII"}]"#)]
fn a_blocked_response_without_usage_metadata_is_an_error(
    #[case] prompt: &str,
    #[case] source: &str,
) {
    let outcome = complete_block(Some(&feedback(prompt)), &candidates(source), None);
    assert!(matches!(
        outcome,
        Err(CandidatesError::Usage(UsageError::MissingUsageMetadata))
    ));
}

#[test]
fn an_unblocked_response_needs_no_usage_metadata() {
    let outcome = complete_block(None, &candidates(r#"[{"finishReason":"STOP"}]"#), None);
    assert!(outcome.unwrap().is_none());
}

#[test]
fn the_streamed_block_is_an_assistant_content_filter_delta() {
    let prompt = feedback(r#"{"blockReason":"PROHIBITED_CONTENT"}"#);
    let choice = stream_block(Some(&prompt)).unwrap();
    assert_eq!(
        rendered(&choice),
        expected(
            r#"{"index":0,"finish_reason":"content_filter","delta":{"role":"assistant","content":null,"reasoning_content":null,"tool_calls":null,"function_call":null,"images":null,"annotations":null,"provider_specific_fields":null},"logprobs":null}"#
        )
    );
}

#[test]
fn no_feedback_means_no_streamed_block() {
    assert!(stream_block(None).is_none());
}
