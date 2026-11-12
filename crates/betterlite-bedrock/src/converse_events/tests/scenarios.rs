use betterlite_models::decisions::Json;

use super::support::{
    FIRST, SECOND, assert_converse, assert_projection, chunk_error, converse_error, empty_delta,
    event, generic, state, text_delta,
};
use crate::converse_events::error::ConverseEventError;

const MODEL: &str = "bedrock/model";

fn wrapped(delta: &str) -> String {
    format!(r#"{{"converse":{delta}}}"#)
}

fn id(uuid: &str) -> String {
    format!("chatcmpl-{uuid}")
}

fn tool_delta(id_value: Option<&str>, name: Option<&str>, arguments: &str, index: i64) -> String {
    let rendered_id = id_value.map_or_else(|| "null".to_owned(), |value| format!(r#""{value}""#));
    let rendered_name = name.map_or_else(|| "null".to_owned(), |value| format!(r#""{value}""#));
    format!(
        r#"{{"content":"","role":"assistant","tool_calls":[{{"id":{rendered_id},"type":"function","function":{{"name":{rendered_name},"arguments":{arguments}}},"index":{index}}}],"provider_specific_fields":null,"thinking_blocks":null,"reasoning_content":null}}"#
    )
}

fn usage(input: i64, output: i64, total: i64, reasoning: Option<i64>, text: Option<i64>) -> String {
    let reasoning_token = reasoning.map_or_else(|| "null".to_owned(), |value| value.to_string());
    let text_token = text.map_or_else(|| "null".to_owned(), |value| value.to_string());
    format!(
        r#"{{"completion_tokens":{output},"prompt_tokens":{input},"total_tokens":{total},"completion_tokens_details":{{"accepted_prediction_tokens":null,"audio_tokens":null,"reasoning_tokens":{reasoning_token},"rejected_prediction_tokens":null,"text_tokens":{text_token},"image_tokens":null,"video_tokens":null}},"prompt_tokens_details":{{"audio_tokens":null,"cache_write_tokens":0,"cached_tokens":0,"text_tokens":{input},"image_tokens":null,"video_tokens":null,"cache_creation_tokens":0}},"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}"#
    )
}

fn usage_chunk(id_value: &str, provider_fields: &str, usage_json: &str) -> String {
    format!(
        r#"{{"id":"{id_value}","model":"{MODEL}","finish_reason":null,"index":0,"delta":{{"content":null,"role":null,"tool_calls":null,"provider_specific_fields":null,"thinking_blocks":null,"reasoning_content":null}},"usage":{usage_json},"provider_specific_fields":{provider_fields}}}"#
    )
}

fn text_chunk(id_value: &str, content: &str) -> String {
    text_delta(id_value, MODEL, content)
}

fn tool_chunk(id_value: &str, call: &str) -> String {
    format!(
        r#"{{"id":"{id_value}","model":"{MODEL}","finish_reason":null,"index":0,"delta":{call},"usage":null,"provider_specific_fields":{{}}}}"#
    )
}

fn finish_chunk(id_value: &str, reason: &str) -> String {
    format!(
        r#"{{"id":"{id_value}","model":"{MODEL}","finish_reason":"{reason}","index":0,"delta":{{"content":"","role":"assistant","tool_calls":null,"provider_specific_fields":null,"thinking_blocks":null,"reasoning_content":null}},"usage":null,"provider_specific_fields":{{}}}}"#
    )
}

fn reasoning_chunk(id_value: &str, blocks: &str, reasoning: &str, provider: &str) -> String {
    format!(
        r#"{{"id":"{id_value}","model":"{MODEL}","finish_reason":null,"index":0,"delta":{{"content":"","role":"assistant","tool_calls":null,"provider_specific_fields":{provider},"thinking_blocks":{blocks},"reasoning_content":{reasoning}}},"usage":null,"provider_specific_fields":{{}}}}"#
    )
}

pub(super) fn message_start_id() {
    conversation_id_is_retained();
    falsey_conversation_id_falls_back_per_chunk();
    nova_wrapper_uses_the_fallback_id();
}

fn conversation_id_is_retained() {
    let mut decoder = state(MODEL, false, &[FIRST, SECOND]);
    assert_eq!(decoder.content_blocks, Vec::<Json>::new());
    assert_eq!(decoder.tool_calls_index, None);
    assert_eq!(decoder.response_id, None);
    assert_eq!(decoder.current_tool_name, None);
    assert!(!decoder.thinking_ran);
    assert_eq!(decoder.provider_reasoning_tokens, None);
    assert!(!decoder.json_mode);
    assert_eq!(decoder.model, MODEL);
    let native = id("abc-123");
    assert_converse(
        &mut decoder,
        r#"{"messageStart": {"conversationId": "abc-123", "role": "assistant"}}"#,
        &empty_delta(&native, MODEL),
    );
    assert_converse(
        &mut decoder,
        r#"{"messageStart": {"conversationId": "later"}}"#,
        &empty_delta(&native, MODEL),
    );
    assert_eq!(decoder.response_id.as_deref(), Some(native.as_str()));
}

fn falsey_conversation_id_falls_back_per_chunk() {
    let mut decoder = state(MODEL, false, &[FIRST, SECOND]);
    assert_converse(
        &mut decoder,
        r#"{"messageStart": {"role": "assistant"}}"#,
        &empty_delta(&id(FIRST), MODEL),
    );
    assert_eq!(decoder.response_id, None);
    assert_converse(
        &mut decoder,
        r#"{"delta": {"text": "x"}, "contentBlockIndex": 0}"#,
        &text_chunk(&id(SECOND), r#""x""#),
    );
    assert_eq!(decoder.response_id.as_deref(), Some(id(SECOND).as_str()));
    for source in [
        r#"{"messageStart": {"conversationId": ""}}"#,
        r#"{"messageStart": {"conversationId": null}}"#,
    ] {
        let mut fresh = state(MODEL, false, &[FIRST]);
        assert_converse(&mut fresh, source, &empty_delta(&id(FIRST), MODEL));
        assert_eq!(fresh.response_id, None);
    }
}

fn nova_wrapper_uses_the_fallback_id() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    assert_projection(
        &mut decoder,
        r#"{"messageStart": {"conversationId": "nova-id"}}"#,
        &wrapped(&empty_delta(&id(FIRST), MODEL)),
    );
    assert_eq!(decoder.response_id.as_deref(), Some(id(FIRST).as_str()));
}

pub(super) fn content_text_delta() {
    delta_priority_and_dispatch_order();
    citations_and_unknown_events();
    content_block_index_conversion();
}

fn delta_priority_and_dispatch_order() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    assert_converse(
        &mut decoder,
        r#"{"delta": {"text": "", "toolUse": {"input": "{\"a\": 1}"}}, "contentBlockIndex": 0}"#,
        &text_chunk(&id(FIRST), r#""""#),
    );
    assert!(decoder.content_blocks.is_empty());
    assert_converse(
        &mut decoder,
        r#"{"start": {}, "delta": {"text": "ignored"}, "contentBlockIndex": 0}"#,
        &text_chunk(&id(FIRST), r#""""#),
    );
    assert_converse(
        &mut decoder,
        r#"{"delta": {"text": "wins"}, "contentBlockIndex": 0}"#,
        &text_chunk(&id(FIRST), r#""wins""#),
    );
    assert_converse(
        &mut decoder,
        r#"{"delta": {"text": null}}"#,
        &text_chunk(&id(FIRST), "null"),
    );
}

fn citations_and_unknown_events() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    let citations = format!(
        r#"{{"id":"{}","model":"{MODEL}","finish_reason":null,"index":0,"delta":{{"content":"","role":"assistant","tool_calls":null,"provider_specific_fields":{{"citationsContent":{{"citations":[]}}}},"thinking_blocks":null,"reasoning_content":null}},"usage":null,"provider_specific_fields":{{}}}}"#,
        id(FIRST)
    );
    assert_converse(
        &mut decoder,
        r#"{"delta": {"citationsContent": {"citations": []}}, "contentBlockIndex": 0}"#,
        &citations,
    );
    assert_converse(
        &mut decoder,
        r#"{"foo": 1}"#,
        &empty_delta(&id(FIRST), MODEL),
    );
    assert_projection(
        &mut decoder,
        r#"{"metrics": {"latencyMs": 5}}"#,
        &wrapped(&empty_delta(&id(FIRST), MODEL)),
    );
}

fn content_block_index_conversion() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    assert_converse(
        &mut decoder,
        r#"{"delta": {"toolUse": {"input": "{}"}}, "contentBlockIndex": 1.9}"#,
        &tool_chunk(&id(FIRST), &tool_delta(None, None, r#""{}""#, 1)),
    );
    assert_converse(
        &mut decoder,
        r#"{"delta": {"toolUse": {"input": "x"}}}"#,
        &tool_chunk(&id(FIRST), &tool_delta(None, None, r#""x""#, 0)),
    );
}

pub(super) fn tool_input_delta() {
    start_resets_the_buffer_and_counts_tools();
    fragments_use_the_stored_index();
    malformed_tool_events_keep_source_state();
    mappings_reverse_tool_names();
}

fn start_resets_the_buffer_and_counts_tools() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    assert_converse(
        &mut decoder,
        r#"{"delta": {"toolUse": {"input": "buffered"}}, "contentBlockIndex": 0}"#,
        &tool_chunk(&id(FIRST), &tool_delta(None, None, r#""buffered""#, 0)),
    );
    assert_eq!(decoder.content_blocks.len(), 1);
    assert_converse(
        &mut decoder,
        r#"{"start": {"toolUse": {"toolUseId": "t1", "name": "first"}}, "contentBlockIndex": 0}"#,
        &tool_chunk(
            &id(FIRST),
            &tool_delta(Some("t1"), Some("first"), r#""""#, 0),
        ),
    );
    assert!(decoder.content_blocks.is_empty());
    assert_converse(
        &mut decoder,
        r#"{"start": {"toolUse": {"toolUseId": "t2", "name": "second"}}, "contentBlockIndex": 1}"#,
        &tool_chunk(
            &id(FIRST),
            &tool_delta(Some("t2"), Some("second"), r#""""#, 1),
        ),
    );
}

fn fragments_use_the_stored_index() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    assert_converse(
        &mut decoder,
        r#"{"start": {"toolUse": {"toolUseId": "t1", "name": "first"}}, "contentBlockIndex": 0}"#,
        &tool_chunk(
            &id(FIRST),
            &tool_delta(Some("t1"), Some("first"), r#""""#, 0),
        ),
    );
    assert_converse(
        &mut decoder,
        r#"{"delta": {"toolUse": {"input": "{\"a\""}}, "contentBlockIndex": 9}"#,
        &tool_chunk(&id(FIRST), &tool_delta(None, None, r#""{\"a\"""#, 0)),
    );
    assert_eq!(decoder.content_blocks.len(), 1);
}

fn malformed_tool_events_keep_source_state() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    assert!(matches!(
        converse_error(
            &mut decoder,
            r#"{"start": {"toolUse": {"name": "broken"}}}"#
        ),
        ConverseEventError::Missing {
            field: "start.toolUse.toolUseId"
        }
    ));
    assert_eq!(decoder.current_tool_name.as_deref(), Some("broken"));
    assert_eq!(decoder.tool_calls_index, Some(0));
    assert!(matches!(
        converse_error(&mut decoder, r#"{"delta": {"toolUse": {}}}"#),
        ConverseEventError::Missing {
            field: "delta.toolUse.input"
        }
    ));
    assert_eq!(decoder.content_blocks.len(), 1);
}

fn mappings_reverse_tool_names() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    decoder
        .mappings
        .insert("bedrock_tool_1".to_owned(), "original".to_owned());
    assert_converse(
        &mut decoder,
        r#"{"start": {"toolUse": {"toolUseId": "t1", "name": "bedrock_tool_1"}}, "contentBlockIndex": 0}"#,
        &tool_chunk(
            &id(FIRST),
            &tool_delta(Some("t1"), Some("original"), r#""""#, 0),
        ),
    );
    let independent = state(MODEL, false, &[SECOND]);
    assert!(independent.content_blocks.is_empty());
    assert_eq!(independent.tool_calls_index, None);
    assert_eq!(independent.current_tool_name, None);
}

pub(super) fn thinking_signature() {
    thinking_blocks_follow_priority();
    start_reasoning_blocks_and_thinking_ran();
    unknown_blocks_leave_thinking_ran_false();
}

fn thinking_blocks_follow_priority() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    assert_converse(
        &mut decoder,
        r#"{"delta": {"reasoningContent": {"text": "think"}}, "contentBlockIndex": 0}"#,
        &reasoning_chunk(
            &id(FIRST),
            r#"[{"type":"thinking","thinking":"think"}]"#,
            r#""think""#,
            r#"{"reasoningContent":{"text":"think"}}"#,
        ),
    );
    assert!(decoder.thinking_ran);
    assert_converse(
        &mut decoder,
        r#"{"delta": {"reasoningContent": {"signature": "sig"}}, "contentBlockIndex": 0}"#,
        &reasoning_chunk(
            &id(FIRST),
            r#"[{"type":"thinking","signature":"sig","thinking":""}]"#,
            r#""""#,
            r#"{"reasoningContent":{"signature":"sig"}}"#,
        ),
    );
    assert_converse(
        &mut decoder,
        r#"{"delta": {"reasoningContent": {"redactedContent": "abc"}}, "contentBlockIndex": 0}"#,
        &reasoning_chunk(
            &id(FIRST),
            r#"[{"type":"redacted_thinking","data":"abc"}]"#,
            r#""""#,
            r#"{"reasoningContent":{"redactedContent":"abc"}}"#,
        ),
    );
}

fn start_reasoning_blocks_and_thinking_ran() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    assert_converse(
        &mut decoder,
        r#"{"start": {"reasoningContent": {"redactedContent": "xyz"}}}"#,
        &reasoning_chunk(
            &id(FIRST),
            r#"[{"type":"redacted_thinking","data":"xyz"}]"#,
            "null",
            r#"{"reasoningContent":{"redactedContent":"xyz"}}"#,
        ),
    );
    assert_converse(
        &mut decoder,
        r#"{"start": {"toolUse": {"toolUseId": "t1", "name": "both"}, "reasoningContent": {"text": "ignored"}}}"#,
        &tool_chunk(
            &id(FIRST),
            &tool_delta(Some("t1"), Some("both"), r#""""#, 0),
        ),
    );
    assert_converse(
        &mut decoder,
        r#"{"start": {"toolUse": null, "reasoningContent": {"text": "selected"}}}"#,
        &reasoning_chunk(
            &id(FIRST),
            r#"[{"type":"thinking","thinking":"selected"}]"#,
            "null",
            r#"{"reasoningContent":{"text":"selected"}}"#,
        ),
    );
    assert_converse(
        &mut decoder,
        r#"{"start": {"reasoningContent": null}}"#,
        &text_chunk(&id(FIRST), r#""""#),
    );
    assert_eq!(decoder.current_tool_name.as_deref(), Some("both"));
    assert_eq!(decoder.tool_calls_index, Some(0));
}

fn unknown_blocks_leave_thinking_ran_false() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    assert_converse(
        &mut decoder,
        r#"{"delta": {"reasoningContent": {"other": 1}}, "contentBlockIndex": 0}"#,
        &reasoning_chunk(
            &id(FIRST),
            "[]",
            "null",
            r#"{"reasoningContent":{"other":1}}"#,
        ),
    );
    assert!(!decoder.thinking_ran);
}

pub(super) fn content_stop_finish() {
    empty_argument_predicate_edges();
    stored_tool_index_wins_over_the_provider_index();
    repeated_stop_repeats_the_empty_chunk();
    stop_reasons_map_and_store_thinking_tokens();
}

fn empty_argument_predicate_edges() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    assert_converse(
        &mut decoder,
        r#"{"contentBlockIndex": 3}"#,
        &text_chunk(&id(FIRST), r#""""#),
    );
    decoder.content_blocks = vec![
        event(r#"{"text": "x"}"#),
        event(r#"{"toolUse": {"input": ""}}"#),
    ];
    assert_converse(
        &mut decoder,
        r#"{"contentBlockIndex": 0}"#,
        &text_chunk(&id(FIRST), r#""""#),
    );
    decoder.content_blocks = vec![event(r#"{"toolUse": {"input": " "}}"#)];
    assert_converse(
        &mut decoder,
        r#"{"contentBlockIndex": 0}"#,
        &text_chunk(&id(FIRST), r#""""#),
    );
    decoder.content_blocks = vec![
        event(r#"{"toolUse": {"input": "a"}}"#),
        event(r#"{"toolUse": {"input": "b"}}"#),
    ];
    assert_converse(
        &mut decoder,
        r#"{"contentBlockIndex": 0}"#,
        &text_chunk(&id(FIRST), r#""""#),
    );
    decoder.content_blocks = Vec::new();
    assert_converse(
        &mut decoder,
        r#"{"contentBlockIndex": 2}"#,
        &text_chunk(&id(FIRST), r#""""#),
    );
}

fn stored_tool_index_wins_over_the_provider_index() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    decoder.content_blocks = vec![event(r#"{"toolUse": {"input": ""}}"#)];
    decoder.tool_calls_index = Some(2);
    assert_converse(
        &mut decoder,
        r#"{"contentBlockIndex": 7}"#,
        &tool_chunk(&id(FIRST), &tool_delta(None, None, r#""{}""#, 2)),
    );
}

fn repeated_stop_repeats_the_empty_chunk() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    decoder.current_tool_name = Some("pending".to_owned());
    decoder.content_blocks = vec![event(r#"{"toolUse": {"input": ""}}"#)];
    let empty_args = tool_chunk(&id(FIRST), &tool_delta(None, None, r#""{}""#, 3));
    assert_converse(&mut decoder, r#"{"contentBlockIndex": 3}"#, &empty_args);
    assert_eq!(decoder.current_tool_name, None);
    assert_converse(&mut decoder, r#"{"contentBlockIndex": 3}"#, &empty_args);
}

fn stop_reasons_map_and_store_thinking_tokens() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    for (source, reason) in [
        (r#"{"stopReason": "end_turn"}"#, "stop"),
        (r#"{"stopReason": "tool_use"}"#, "tool_calls"),
        (r#"{"stopReason": "max_tokens"}"#, "length"),
        (
            r#"{"stopReason": "guardrail_intervened"}"#,
            "content_filter",
        ),
        (r#"{"stopReason": "unknown-value"}"#, "stop"),
        (r#"{"stopReason": null}"#, "stop"),
    ] {
        assert_converse(&mut decoder, source, &finish_chunk(&id(FIRST), reason));
    }
    assert_converse(
        &mut decoder,
        r#"{"stopReason": "end_turn", "additionalModelResponseFields": {"usage": {"output_tokens_details": {"thinking_tokens": 7}}}}"#,
        &finish_chunk(&id(FIRST), "stop"),
    );
    assert_eq!(decoder.provider_reasoning_tokens, Some(7));
    assert_converse(
        &mut decoder,
        r#"{"usage": {"inputTokens": 1, "outputTokens": 5}}"#,
        &usage_chunk(&id(FIRST), "{}", &usage(1, 5, 6, Some(5), Some(0))),
    );
    assert_eq!(decoder.provider_reasoning_tokens, Some(7));
}

pub(super) fn metadata_usage() {
    usage_only_and_trace_fields();
    thinking_ran_and_provider_tokens_shape_usage();
    nova_metadata_usage();
}

fn usage_only_and_trace_fields() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    assert_converse(
        &mut decoder,
        r#"{"usage": {"inputTokens": 1, "outputTokens": 2}}"#,
        &usage_chunk(&id(FIRST), "{}", &usage(1, 2, 3, Some(0), Some(2))),
    );
    let traced = format!(
        r#"{{"id":"{}","model":"{MODEL}","finish_reason":null,"index":0,"delta":{{"content":"","role":"assistant","tool_calls":null,"provider_specific_fields":null,"thinking_blocks":null,"reasoning_content":null}},"usage":{},"provider_specific_fields":{{"trace":{{"guardrail":{{}}}}}}}}"#,
        id(FIRST),
        usage(1, 2, 3, Some(0), Some(2))
    );
    assert_converse(
        &mut decoder,
        r#"{"usage": {"inputTokens": 1, "outputTokens": 2}, "trace": {"guardrail": {}}}"#,
        &traced,
    );
    assert_converse(
        &mut decoder,
        r#"{"usage": {"inputTokens": 1, "outputTokens": 2}, "trace": {}}"#,
        &usage_chunk(&id(FIRST), "{}", &usage(1, 2, 3, Some(0), Some(2))),
    );
    assert_converse(
        &mut decoder,
        r#"{"usage": {"inputTokens": 1, "outputTokens": 2}, "trace": false}"#,
        &usage_chunk(&id(FIRST), "{}", &usage(1, 2, 3, Some(0), Some(2))),
    );
}

fn thinking_ran_and_provider_tokens_shape_usage() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    assert_converse(
        &mut decoder,
        r#"{"delta": {"reasoningContent": {"signature": "sig"}}, "contentBlockIndex": 0}"#,
        &reasoning_chunk(
            &id(FIRST),
            r#"[{"type":"thinking","signature":"sig","thinking":""}]"#,
            r#""""#,
            r#"{"reasoningContent":{"signature":"sig"}}"#,
        ),
    );
    assert_converse(
        &mut decoder,
        r#"{"usage": {"inputTokens": 4, "outputTokens": 9, "totalTokens": 13}}"#,
        &usage_chunk(&id(FIRST), "{}", &usage(4, 9, 13, None, None)),
    );
    assert_converse(
        &mut decoder,
        r#"{"stopReason": "end_turn", "additionalModelResponseFields": {"usage": {"output_tokens_details": {"thinking_tokens": 7}}}}"#,
        &finish_chunk(&id(FIRST), "stop"),
    );
    assert_converse(
        &mut decoder,
        r#"{"usage": {"inputTokens": 1, "outputTokens": 5}}"#,
        &usage_chunk(&id(FIRST), "{}", &usage(1, 5, 6, Some(5), Some(0))),
    );
}

fn nova_metadata_usage() {
    let mut decoder = state(MODEL, false, &[SECOND]);
    let counted = r#"{"completion_tokens":3,"prompt_tokens":12267,"total_tokens":12270,"completion_tokens_details":{"accepted_prediction_tokens":null,"audio_tokens":null,"reasoning_tokens":0,"rejected_prediction_tokens":null,"text_tokens":3,"image_tokens":null,"video_tokens":null},"prompt_tokens_details":{"audio_tokens":null,"cache_write_tokens":0,"cached_tokens":12262,"text_tokens":5,"image_tokens":null,"video_tokens":null,"cache_creation_tokens":0},"cache_creation_input_tokens":0,"cache_read_input_tokens":12262}"#;
    assert_projection(
        &mut decoder,
        r#"{"metadata": {"usage": {"inputTokens": 5, "outputTokens": 3, "cacheReadInputTokenCount": 12262, "cacheWriteInputTokenCount": 0}}}"#,
        &wrapped(&usage_chunk(&id(SECOND), "{}", counted)),
    );
}

pub(super) fn invoke_chunk_provider_dispatch() {
    base_provider_branches();
    outputs_branches();
    plain_text_branches();
    ai21_text_chunks();
    nova_wrapper_selection();
}

fn base_provider_branches() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    assert_projection(
        &mut decoder,
        r#"{"outputText": "plain"}"#,
        &generic(r#""plain""#, false, r#""""#),
    );
    assert_projection(
        &mut decoder,
        r#"{"delta": {"text": "x"}, "contentBlockIndex": 0}"#,
        &wrapped(&text_chunk(&id(FIRST), r#""x""#)),
    );
    assert_projection(
        &mut decoder,
        r#"{"metrics": {}}"#,
        &wrapped(&empty_delta(&id(FIRST), MODEL)),
    );
    assert_projection(
        &mut decoder,
        r#"{"trace": null}"#,
        &wrapped(&empty_delta(&id(FIRST), MODEL)),
    );
}

fn outputs_branches() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    assert_projection(
        &mut decoder,
        r#"{"outputs": [{"text": "mistral"}], "stop_reason": "stop"}"#,
        &generic(r#""mistral""#, true, r#""stop""#),
    );
    assert_projection(
        &mut decoder,
        r#"{"outputs": [], "stop_reason": null}"#,
        &generic(r#""""#, false, r#""""#),
    );
    assert_projection(
        &mut decoder,
        r#"{"outputs": [{"text": null}], "stop_reason": "length"}"#,
        &generic(r#""""#, true, r#""length""#),
    );
    assert_projection(
        &mut decoder,
        r#"{"outputs": [{"text": "a"}, {"text": "b"}], "stop_reason": ""}"#,
        &generic(r#""""#, true, r#""""#),
    );
}

fn plain_text_branches() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    assert_projection(
        &mut decoder,
        r#"{"generation": "meta"}"#,
        &generic(r#""meta""#, false, r#""""#),
    );
    assert_projection(
        &mut decoder,
        r#"{"text": "cohere"}"#,
        &generic(r#""cohere""#, false, r#""""#),
    );
    assert_projection(
        &mut decoder,
        r#"{"finish_reason": "complete"}"#,
        &generic(r#""""#, true, r#""complete""#),
    );
    assert_projection(
        &mut decoder,
        r#"{"completionReason": "FINISH"}"#,
        &generic(r#""""#, true, r#""FINISH""#),
    );
    assert_projection(
        &mut decoder,
        r#"{"completionReason": ""}"#,
        &generic(r#""""#, false, r#""""#),
    );
    assert_projection(
        &mut decoder,
        r#"{"usage": {"inputTokens": 1, "outputTokens": 2}}"#,
        &generic(r#""""#, false, r#""""#),
    );
    assert_projection(
        &mut decoder,
        r#"{"unknown": "value"}"#,
        &generic(r#""""#, false, r#""""#),
    );
}

fn ai21_text_chunks() {
    let mut decoder = state("ai21.j2", false, &[SECOND]);
    assert_projection(
        &mut decoder,
        r#"{"completions": [{"data": {"text": "a21"}}]}"#,
        &generic(r#""a21""#, true, r#""stop""#),
    );
}

fn nova_wrapper_selection() {
    let mut decoder = state(MODEL, false, &[SECOND]);
    assert_projection(
        &mut decoder,
        r#"{"messageStart": {"conversationId": "nova-id"}}"#,
        &wrapped(&empty_delta(&id(SECOND), MODEL)),
    );
    assert_projection(
        &mut decoder,
        r#"{"contentBlockStart": {"start": {}, "contentBlockIndex": 0}, "messageStart": {"role": "assistant"}}"#,
        &wrapped(&empty_delta(&id(SECOND), MODEL)),
    );
    assert_projection(
        &mut decoder,
        r#"{"messageStop": {"stopReason": "end_turn"}}"#,
        &wrapped(&finish_chunk(&id(SECOND), "stop")),
    );
}

pub(super) fn json_tool_stream() {
    json_mode_suppresses_the_internal_tool();
    non_tool_start_keeps_the_restored_name();
    json_mode_off_emits_the_tool();
    json_mode_keeps_other_tools();
}

fn json_mode_keeps_other_tools() {
    let mut decoder = state(MODEL, true, &[FIRST]);
    assert_converse(
        &mut decoder,
        r#"{"start": {"toolUse": {"toolUseId": "w", "name": "get_weather"}}, "contentBlockIndex": 0}"#,
        &tool_chunk(
            &id(FIRST),
            &tool_delta(Some("w"), Some("get_weather"), r#""""#, 0),
        ),
    );
    assert_converse(
        &mut decoder,
        r#"{"delta": {"toolUse": {"input": ""}}, "contentBlockIndex": 0}"#,
        &tool_chunk(&id(FIRST), &tool_delta(None, None, r#""""#, 0)),
    );
    assert_converse(
        &mut decoder,
        r#"{"contentBlockIndex": 0}"#,
        &tool_chunk(&id(FIRST), &tool_delta(None, None, r#""{}""#, 0)),
    );
    assert_eq!(decoder.current_tool_name, None);
}

fn json_mode_suppresses_the_internal_tool() {
    let mut decoder = state(MODEL, true, &[FIRST]);
    assert_converse(
        &mut decoder,
        r#"{"start": {"toolUse": {"toolUseId": "j", "name": "json_tool_call"}}, "contentBlockIndex": 0}"#,
        &text_chunk(&id(FIRST), r#""""#),
    );
    assert_eq!(decoder.tool_calls_index, None);
    assert_eq!(decoder.current_tool_name.as_deref(), Some("json_tool_call"));
    assert_converse(
        &mut decoder,
        r#"{"delta": {"toolUse": {"input": "{\"a\": 1}"}}, "contentBlockIndex": 0}"#,
        &text_chunk(&id(FIRST), r#""{\"a\": 1}""#),
    );
    assert_eq!(decoder.content_blocks.len(), 1);
    assert_converse(
        &mut decoder,
        r#"{"contentBlockIndex": 0}"#,
        &text_chunk(&id(FIRST), r#""""#),
    );
    assert_eq!(decoder.current_tool_name, None);
}

fn non_tool_start_keeps_the_restored_name() {
    let mut decoder = state(MODEL, true, &[FIRST]);
    assert_converse(
        &mut decoder,
        r#"{"start": {"toolUse": {"toolUseId": "j", "name": "json_tool_call"}}, "contentBlockIndex": 0}"#,
        &text_chunk(&id(FIRST), r#""""#),
    );
    assert_converse(
        &mut decoder,
        r#"{"start": {}}"#,
        &text_chunk(&id(FIRST), r#""""#),
    );
    assert_eq!(decoder.current_tool_name.as_deref(), Some("json_tool_call"));
    assert!(decoder.content_blocks.is_empty());
    assert_converse(
        &mut decoder,
        r#"{"delta": {"toolUse": {"input": "{}"}}, "contentBlockIndex": 0}"#,
        &text_chunk(&id(FIRST), r#""{}""#),
    );
}

fn json_mode_off_emits_the_tool() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    assert_converse(
        &mut decoder,
        r#"{"start": {"toolUse": {"toolUseId": "j", "name": "json_tool_call"}}, "contentBlockIndex": 0}"#,
        &tool_chunk(
            &id(FIRST),
            &tool_delta(Some("j"), Some("json_tool_call"), r#""""#, 0),
        ),
    );
    assert_eq!(decoder.tool_calls_index, Some(0));
    assert_converse(
        &mut decoder,
        r#"{"delta": {"toolUse": {"input": ""}}, "contentBlockIndex": 0}"#,
        &tool_chunk(&id(FIRST), &tool_delta(None, None, r#""""#, 0)),
    );
    assert_converse(
        &mut decoder,
        r#"{"contentBlockIndex": 0}"#,
        &tool_chunk(&id(FIRST), &tool_delta(None, None, r#""{}""#, 0)),
    );
}

pub(super) fn converse_projection_error() {
    converse_failures();
    dispatch_failures();
    message_start_failure();
    streaming_error_text();
}

fn converse_failures() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    assert!(matches!(
        converse_error(&mut decoder, r#"{"delta": {"toolUse": {}}}"#),
        ConverseEventError::Missing {
            field: "delta.toolUse.input"
        }
    ));
    assert_eq!(decoder.content_blocks.len(), 1);
    assert!(matches!(
        converse_error(&mut decoder, r#"{"usage": {}}"#),
        ConverseEventError::Usage(_)
    ));
    assert!(matches!(
        converse_error(&mut decoder, r#"{"usage": null}"#),
        ConverseEventError::Usage(_)
    ));
    assert!(matches!(
        converse_error(&mut decoder, r#"{"start": 5}"#),
        ConverseEventError::FieldType { field: "start" }
    ));
    assert!(matches!(
        converse_error(&mut decoder, r#"{"delta": []}"#),
        ConverseEventError::FieldType { field: "delta" }
    ));
    assert!(matches!(
        converse_error(&mut decoder, r#"{"contentBlockIndex": []}"#),
        ConverseEventError::FieldType {
            field: "contentBlockIndex"
        }
    ));
}

fn dispatch_failures() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    assert!(matches!(
        chunk_error(&mut decoder, r#"{"outputs": 5}"#),
        ConverseEventError::FieldType { field: "outputs" }
    ));
    assert!(matches!(
        chunk_error(&mut decoder, r#"{"outputs": [5]}"#),
        ConverseEventError::FieldType {
            field: "outputs[0]"
        }
    ));
    assert!(matches!(
        chunk_error(&mut decoder, r#"{"metadata": 5}"#),
        ConverseEventError::FieldType {
            field: "nova event payload"
        }
    ));
    let mut ai21 = state("ai21.j2", false, &[FIRST]);
    assert!(matches!(
        chunk_error(&mut ai21, r#"{"completions": 5}"#),
        ConverseEventError::FieldType {
            field: "completions"
        }
    ));
    assert!(matches!(
        chunk_error(&mut ai21, r#"{"completions": []}"#),
        ConverseEventError::Missing {
            field: "completions[0]"
        }
    ));
}

fn streaming_error_text() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    let direct = decoder
        .converse_chunk_parser(&event(r#"{"delta": {"toolUse": {}}}"#))
        .unwrap_err();
    assert_eq!(
        direct.to_string(),
        "Received streaming error - the Bedrock event field delta.toolUse.input is required"
    );
    let routed = decoder
        .chunk_parser(&event(r#"{"contentBlockIndex": []}"#))
        .unwrap_err();
    assert!(matches!(routed, ConverseEventError::Streaming(_)));
    let outer = chunk_error(&mut decoder, r#"{"outputs": 5}"#);
    assert!(!outer.to_string().starts_with("Received streaming error"));
}

fn message_start_failure() {
    let mut decoder = state(MODEL, false, &[FIRST]);
    assert!(matches!(
        converse_error(&mut decoder, r#"{"messageStart": 5}"#),
        ConverseEventError::FieldType {
            field: "messageStart"
        }
    ));
}
