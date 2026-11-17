#![cfg(test)]

use uuid::Uuid;

use crate::converse_response::wire::ReplyToolCall;

use super::support::{SECOND_ID, convert, fixture, json, optional_params, rendered};

pub(super) fn tool_calls_response() {
    content_tool_calls_keep_their_block_index();
    recovered_calls_take_the_next_supplied_id();
    recovery_only_matches_an_offered_tool();
    json_mode_filters_synthetic_calls();
}

fn content_tool_calls_keep_their_block_index() {
    let mut fixture = fixture();
    fixture.optional = optional_params(
        r#"{"tools": [{"toolSpec": {"name": "get_weather", "inputSchema": {"json": {"type": "object"}}}}]}"#,
    );
    let body = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "Let me check."}, {"toolUse": {"toolUseId": "tooluse_abc", "name": "get_weather", "input": {"city": "Paris"}}}]}}, "stopReason": "tool_use", "usage": {"inputTokens": 10, "outputTokens": 5, "totalTokens": 15}}"#;
    let run = convert(&fixture, body);
    assert_eq!(run.finish(), "tool_calls");
    assert_eq!(run.message().content.as_deref(), Some("Let me check."));
    let calls = run.message().tool_calls.as_deref().expect("a tool call");
    assert_eq!(calls.len(), 1);
    let ReplyToolCall::Content(call) = &calls[0] else {
        panic!("a content tool call keeps its wire shape");
    };
    assert_eq!(call.index, 1);
    assert_eq!(call.id.as_deref(), Some("tooluse_abc"));
    assert_eq!(call.kind, "function");
    assert_eq!(call.function.name.as_deref(), Some("get_weather"));
    assert_eq!(
        call.function.arguments.as_deref(),
        Some(r#"{"city": "Paris"}"#)
    );
}

fn recovered_calls_take_the_next_supplied_id() {
    recovered_call_keeps_its_arguments();
    a_recovered_call_without_arguments_is_empty();
}

fn recovered_call_keeps_its_arguments() {
    let mut fixture = fixture();
    fixture.optional = optional_params(r#"{"tools": [{"toolSpec": {"name": "real"}}]}"#);
    fixture.ids = vec![Uuid::parse_str(SECOND_ID).expect("the fixture id parses")];
    let body = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "{\"type\": \"function\", \"name\": \"real\", \"arguments\": {\"b\": 1, \"a\": 2}}"}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let run = convert(&fixture, body);
    assert_eq!(run.finish(), "tool_calls");
    assert!(run.message().content.is_none());
    let calls = run.message().tool_calls.as_deref().expect("a tool call");
    assert_eq!(calls.len(), 1);
    let ReplyToolCall::Recovered(call) = &calls[0] else {
        panic!("a recovered call keeps its own wire shape");
    };
    assert_eq!(call.id, SECOND_ID);
    assert_eq!(call.kind, "function");
    assert_eq!(call.function.name.as_deref(), Some("real"));
    assert_eq!(call.function.arguments, r#"{"b": 1, "a": 2}"#);
}

fn a_recovered_call_without_arguments_is_empty() {
    let mut fixture = fixture();
    fixture.optional = optional_params(r#"{"tools": [{"function": {"name": "real"}}]}"#);
    fixture.ids = vec![Uuid::parse_str(SECOND_ID).expect("the fixture id parses")];
    let body = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "{\"type\": \"function\", \"name\": \"real\"}"}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let run = convert(&fixture, body);
    assert_eq!(run.finish(), "tool_calls");
    assert!(run.message().content.is_none());
    let calls = run.message().tool_calls.as_deref().expect("a tool call");
    let ReplyToolCall::Recovered(call) = &calls[0] else {
        panic!("a recovered call keeps its own wire shape");
    };
    assert_eq!(call.function.arguments, "");
}

fn recovery_only_matches_an_offered_tool() {
    let mut fixture = fixture();
    fixture.optional = optional_params(r#"{"tools": [{"toolSpec": {"name": "real"}}]}"#);
    fixture.ids = vec![Uuid::parse_str(SECOND_ID).expect("the fixture id parses")];
    let body = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "{\"type\": \"function\", \"name\": \"other\", \"arguments\": \"{}\"}"}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let run = convert(&fixture, body);
    assert_eq!(run.finish(), "stop");
    assert!(run.message().tool_calls.is_none());
    assert_eq!(
        run.message().content.as_deref(),
        Some(r#"{"type": "function", "name": "other", "arguments": "{}"}"#)
    );
}

fn json_mode_filters_synthetic_calls() {
    an_explicit_false_json_mode_suppresses_the_option();
    a_json_tool_call_survives_without_json_mode();
    let mut mixed = fixture();
    mixed.optional = optional_params(
        r#"{"tools": [{"toolSpec": {"name": "real", "inputSchema": {"json": {"type": "object"}}}}]}"#,
    );
    mixed.json_mode = Some(true);
    let mixed_body = r#"{"output": {"message": {"role": "assistant", "content": [{"toolUse": {"toolUseId": "tooluse_real", "name": "real", "input": {"x": 1}}}, {"toolUse": {"toolUseId": "tooluse_json", "name": "json_tool_call", "input": {"answer": 42}}}, {"toolUse": {"toolUseId": "tooluse_late", "name": "real", "input": {"z": 3}}}]}}, "stopReason": "tool_use", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let mixed_run = convert(&mixed, mixed_body);
    assert_eq!(mixed_run.finish(), "tool_calls");
    assert_eq!(
        mixed_run.message().content.as_deref(),
        Some(r#"{"answer": 42}"#)
    );
    assert_eq!(
        rendered(&mixed_run.message().tool_calls),
        json(
            r#"[{"index": 0, "function": {"arguments": "{\"x\": 1}", "name": "real"}, "id": "tooluse_real", "type": "function"}, {"index": 2, "function": {"arguments": "{\"z\": 3}", "name": "real"}, "id": "tooluse_late", "type": "function"}]"#
        )
    );

    let mut synthetic = fixture();
    synthetic.optional = optional_params(r#"{"tools": [{"toolSpec": {"name": "real"}}]}"#);
    synthetic.json_mode = Some(true);
    let synthetic_body = r#"{"output": {"message": {"role": "assistant", "content": [{"toolUse": {"toolUseId": "tooluse_json", "name": "json_tool_call", "input": {"properties": {"answer": 42}}}}]}}, "stopReason": "tool_use", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let synthetic_run = convert(&synthetic, synthetic_body);
    assert_eq!(synthetic_run.finish(), "stop");
    assert!(synthetic_run.message().tool_calls.is_none());
    assert_eq!(
        synthetic_run.message().content.as_deref(),
        Some(r#"{"answer": 42}"#)
    );
}

pub(super) fn an_explicit_false_json_mode_suppresses_the_option() {
    let mut fixture = fixture();
    fixture.optional =
        optional_params(r#"{"json_mode": true, "tools": [{"toolSpec": {"name": "real"}}]}"#);
    fixture.json_mode = Some(false);
    let body = r#"{"output": {"message": {"content": [{"toolUse": {"toolUseId": "tooluse_json", "name": "json_tool_call", "input": {"answer": 42}}}]}}, "stopReason": "tool_use", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let run = convert(&fixture, body);
    assert_eq!(run.finish(), "tool_calls");
    assert_eq!(run.message().content.as_deref(), Some(""));
    assert_eq!(
        rendered(&run.message().tool_calls),
        json(
            r#"[{"index": 0, "function": {"arguments": "{\"answer\": 42}", "name": "json_tool_call"}, "id": "tooluse_json", "type": "function"}]"#
        )
    );
}

fn a_json_tool_call_survives_without_json_mode() {
    let mut fixture = fixture();
    fixture.optional = optional_params(r#"{"tools": [{"toolSpec": {"name": "real"}}]}"#);
    let body = r#"{"output": {"message": {"role": "assistant", "content": [{"toolUse": {"toolUseId": "tooluse_json", "name": "json_tool_call", "input": {"answer": 42}}}]}}, "stopReason": "tool_use", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let run = convert(&fixture, body);
    assert_eq!(run.finish(), "tool_calls");
    assert_eq!(run.message().content.as_deref(), Some(""));
    assert_eq!(
        rendered(&run.message().tool_calls),
        json(
            r#"[{"index": 0, "function": {"arguments": "{\"answer\": 42}", "name": "json_tool_call"}, "id": "tooluse_json", "type": "function"}]"#
        )
    );
}

pub(super) fn citation_content_alignment() {
    whitespace_content_is_replaced_by_the_cited_text();
    punctuation_content_gets_the_cited_text_prepended();
    alphanumeric_content_keeps_its_own_text();
}

const PARIS_CITATION: &str = r#"[{"type": "url_citation", "url_citation": {"end_index": 5, "start_index": 0, "title": "Paris", "url": "https://example.test/paris"}}]"#;

fn whitespace_content_is_replaced_by_the_cited_text() {
    let body = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "  "}, {"citationsContent": {"content": [{"text": "Paris is the capital."}], "citations": [{"location": {"searchResultLocation": {"start": 0, "end": 5}}, "source": "https://example.test/paris", "title": "Paris"}]}}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let run = convert(&fixture(), body);
    assert_eq!(
        run.message().content.as_deref(),
        Some("Paris is the capital.")
    );
    assert_eq!(rendered(&run.message().annotations), json(PARIS_CITATION));
    let blocks = run
        .message()
        .provider_specific_fields
        .as_ref()
        .expect("the raw citation blocks");
    assert_eq!(
        blocks.get("citationsContent"),
        Some(&json(
            r#"[{"content": [{"text": "Paris is the capital."}], "citations": [{"location": {"searchResultLocation": {"start": 0, "end": 5}}, "source": "https://example.test/paris", "title": "Paris"}]}]"#
        ))
    );

    let separator = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "\u001c"}, {"citationsContent": {"content": [{"text": "Cited"}], "citations": []}}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let separator_run = convert(&fixture(), separator);
    assert_eq!(separator_run.message().content.as_deref(), Some("Cited"));
    assert!(separator_run.message().annotations.is_none());
}

fn punctuation_content_gets_the_cited_text_prepended() {
    let body = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "... "}, {"citationsContent": {"content": [{"text": "per cent"}], "citations": [{"location": {"searchResultLocation": {"start": 1, "end": 4}}, "source": null, "title": null}]}}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let run = convert(&fixture(), body);
    assert_eq!(run.message().content.as_deref(), Some("per cent... "));
    assert_eq!(
        rendered(&run.message().annotations),
        json(
            r#"[{"type": "url_citation", "url_citation": {"end_index": 4, "start_index": 1, "title": "", "url": ""}}]"#
        )
    );
}

fn alphanumeric_content_keeps_its_own_text() {
    let same = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "Same words"}, {"citationsContent": {"content": [{"text": "Same words"}], "citations": [{"location": {"searchResultLocation": {"start": 0, "end": 4}}, "source": "https://s.test", "title": "T"}]}}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let same_run = convert(&fixture(), same);
    assert_eq!(same_run.message().content.as_deref(), Some("Same words"));
    assert!(same_run.message().annotations.is_none());

    let longer = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "Real answer. "}, {"citationsContent": {"content": [{"text": "Cited sentence."}], "citations": [{"location": {"searchResultLocation": {"start": 0, "end": 16}}, "source": "https://example.test/cited", "title": "Cited"}]}}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let longer_run = convert(&fixture(), longer);
    assert_eq!(
        longer_run.message().content.as_deref(),
        Some("Real answer. ")
    );
    assert!(longer_run.message().annotations.is_none());

    let uncited = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "  "}, {"citationsContent": {"content": [{"text": "Cited"}], "citations": []}}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let uncited_run = convert(&fixture(), uncited);
    assert_eq!(uncited_run.message().content.as_deref(), Some("Cited"));
    assert!(uncited_run.message().annotations.is_none());

    let accented = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "\u00e9"}, {"citationsContent": {"content": [{"text": "cited text"}], "citations": [{"location": {"searchResultLocation": {"start": 0, "end": 4}}, "source": "https://example.test/accent", "title": "Accent"}]}}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let accented_run = convert(&fixture(), accented);
    assert_eq!(accented_run.message().content.as_deref(), Some("\u{e9}"));
    assert!(accented_run.message().annotations.is_none());

    let digit = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "\u0663"}, {"citationsContent": {"content": [{"text": "cited text"}], "citations": [{"location": {"searchResultLocation": {"start": 0, "end": 4}}, "source": "https://example.test/digit", "title": "Digit"}]}}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let digit_run = convert(&fixture(), digit);
    assert_eq!(digit_run.message().content.as_deref(), Some("\u{663}"));
    assert!(digit_run.message().annotations.is_none());
}
