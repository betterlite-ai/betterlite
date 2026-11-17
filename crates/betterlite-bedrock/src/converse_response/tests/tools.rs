#![cfg(test)]

use uuid::Uuid;

use crate::converse_response::wire::ReplyToolCall;

use super::support::{Fixture, SECOND_ID, json, optional_params, rendered, run};

pub(super) fn tool_calls_response() {
    content_tool_calls_keep_their_block_index();
    recovered_calls_take_the_next_supplied_id();
    recovery_only_matches_an_offered_tool();
    json_mode_filters_synthetic_calls();
}

fn content_tool_calls_keep_their_block_index() {
    let fixture = Fixture {
        optional: optional_params(
            r#"{"tools": [{"toolSpec": {"name": "get_weather", "inputSchema": {"json": {"type": "object"}}}}]}"#,
        ),
        ..Fixture::default()
    };
    let body = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "Let me check."}, {"toolUse": {"toolUseId": "tooluse_abc", "name": "get_weather", "input": {"city": "Paris"}}}]}}, "stopReason": "tool_use", "usage": {"inputTokens": 10, "outputTokens": 5, "totalTokens": 15}}"#;
    let run = run(&fixture, body);
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
    let fixture = Fixture {
        optional: optional_params(r#"{"tools": [{"toolSpec": {"name": "real"}}]}"#),
        ids: vec![Uuid::parse_str(SECOND_ID).expect("the fixture id parses")],
        ..Fixture::default()
    };
    let body = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "{\"type\": \"function\", \"name\": \"real\", \"arguments\": {\"b\": 1, \"a\": 2}}"}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let run = run(&fixture, body);
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
    let fixture = Fixture {
        optional: optional_params(r#"{"tools": [{"function": {"name": "real"}}]}"#),
        ids: vec![Uuid::parse_str(SECOND_ID).expect("the fixture id parses")],
        ..Fixture::default()
    };
    let body = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "{\"type\": \"function\", \"name\": \"real\"}"}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let run = run(&fixture, body);
    assert_eq!(run.finish(), "tool_calls");
    assert!(run.message().content.is_none());
    let calls = run.message().tool_calls.as_deref().expect("a tool call");
    let ReplyToolCall::Recovered(call) = &calls[0] else {
        panic!("a recovered call keeps its own wire shape");
    };
    assert_eq!(call.function.arguments, "");
}

fn recovery_only_matches_an_offered_tool() {
    let fixture = Fixture {
        optional: optional_params(r#"{"tools": [{"toolSpec": {"name": "real"}}]}"#),
        ids: vec![Uuid::parse_str(SECOND_ID).expect("the fixture id parses")],
        ..Fixture::default()
    };
    let body = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "{\"type\": \"function\", \"name\": \"other\", \"arguments\": \"{}\"}"}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let run = run(&fixture, body);
    assert_eq!(run.finish(), "stop");
    assert!(run.message().tool_calls.is_none());
    assert_eq!(
        run.message().content.as_deref(),
        Some(r#"{"type": "function", "name": "other", "arguments": "{}"}"#)
    );
}

fn json_mode_filters_synthetic_calls() {
    an_explicit_false_json_mode_suppresses_the_option();
    let mixed = Fixture {
        optional: optional_params(
            r#"{"tools": [{"toolSpec": {"name": "real", "inputSchema": {"json": {"type": "object"}}}}]}"#,
        ),
        json_mode: Some(true),
        ..Fixture::default()
    };
    let body = r#"{"output": {"message": {"role": "assistant", "content": [{"toolUse": {"toolUseId": "tooluse_real", "name": "real", "input": {"x": 1}}}, {"toolUse": {"toolUseId": "tooluse_json", "name": "json_tool_call", "input": {"answer": 42}}}, {"toolUse": {"toolUseId": "tooluse_late", "name": "real", "input": {"z": 3}}}]}}, "stopReason": "tool_use", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let run = run(&mixed, body);
    assert_eq!(run.finish(), "tool_calls");
    assert_eq!(run.message().content.as_deref(), Some(r#"{"answer": 42}"#));
    assert_eq!(
        rendered(&run.message().tool_calls),
        json(
            r#"[{"index": 0, "function": {"arguments": "{\"x\": 1}", "name": "real"}, "id": "tooluse_real", "type": "function"}, {"index": 2, "function": {"arguments": "{\"z\": 3}", "name": "real"}, "id": "tooluse_late", "type": "function"}]"#
        )
    );

    let synthetic = Fixture {
        optional: optional_params(r#"{"tools": [{"toolSpec": {"name": "real"}}]}"#),
        json_mode: Some(true),
        ..Fixture::default()
    };
    let body = r#"{"output": {"message": {"role": "assistant", "content": [{"toolUse": {"toolUseId": "tooluse_json", "name": "json_tool_call", "input": {"properties": {"answer": 42}}}}]}}, "stopReason": "tool_use", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let run = run(&synthetic, body);
    assert_eq!(run.finish(), "stop");
    assert!(run.message().tool_calls.is_none());
    assert_eq!(run.message().content.as_deref(), Some(r#"{"answer": 42}"#));
}

pub(super) fn an_explicit_false_json_mode_suppresses_the_option() {
    let fixture = Fixture {
        optional: optional_params(
            r#"{"json_mode": true, "tools": [{"toolSpec": {"name": "real"}}]}"#,
        ),
        json_mode: Some(false),
        ..Fixture::default()
    };
    let body = r#"{"output": {"message": {"content": [{"toolUse": {"toolUseId": "tooluse_json", "name": "json_tool_call", "input": {"answer": 42}}}]}}, "stopReason": "tool_use", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let run = run(&fixture, body);
    assert_eq!(run.finish(), "tool_calls");
    assert_eq!(run.message().content.as_deref(), Some(""));
    assert_eq!(
        rendered(&run.message().tool_calls),
        json(
            r#"[{"index": 0, "function": {"arguments": "{\"answer\": 42}", "name": "json_tool_call"}, "id": "tooluse_json", "type": "function"}]"#
        )
    );
}

fn citation_content_alignment() {
    whitespace_content_is_replaced_by_the_cited_text();
    punctuation_content_gets_the_cited_text_prepended();
    alphanumeric_content_keeps_its_own_text();
}

const PARIS_CITATION: &str = r#"[{"type": "url_citation", "url_citation": {"end_index": 5, "start_index": 0, "title": "Paris", "url": "https://example.test/paris"}}]"#;

fn whitespace_content_is_replaced_by_the_cited_text() {
    let body = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "  "}, {"citationsContent": {"content": [{"text": "Paris is the capital."}], "citations": [{"location": {"searchResultLocation": {"start": 0, "end": 5}}, "source": "https://example.test/paris", "title": "Paris"}]}}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let run = run(&Fixture::default(), body);
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
}

fn punctuation_content_gets_the_cited_text_prepended() {
    let body = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "... "}, {"citationsContent": {"content": [{"text": "per cent"}], "citations": [{"location": {"searchResultLocation": {"start": 1, "end": 4}}, "source": null, "title": null}]}}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let run = run(&Fixture::default(), body);
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
    let run = run(&Fixture::default(), same);
    assert_eq!(run.message().content.as_deref(), Some("Same words"));
    assert!(run.message().annotations.is_none());

    let longer = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "Real answer. "}, {"citationsContent": {"content": [{"text": "Cited sentence."}], "citations": [{"location": {"searchResultLocation": {"start": 0, "end": 16}}, "source": "https://example.test/cited", "title": "Cited"}]}}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let run = run(&Fixture::default(), longer);
    assert_eq!(run.message().content.as_deref(), Some("Real answer. "));
    assert!(run.message().annotations.is_none());

    let uncited = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "  "}, {"citationsContent": {"content": [{"text": "Cited"}], "citations": []}}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let run = run(&Fixture::default(), uncited);
    assert_eq!(run.message().content.as_deref(), Some("Cited"));
    assert!(run.message().annotations.is_none());
}
