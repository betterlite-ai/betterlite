#![cfg(test)]

use betterlite_models::decisions::Json;
use chrono::{FixedOffset, NaiveDate, TimeZone as _};
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::LlmResponse;
use super::content::{
    GalileoKwargs, Level, ResponseObj, galileo_input_output_content, serialize_galileo_output,
};
use super::messages::{
    convert_content_list_to_str, galileo_input_messages, input_text_from_messages,
};
use super::metrics::{TokenMetrics, token_metrics_from_record, token_metrics_json};
use super::support::has_value;
use super::timestamps::{RawTimestamp, format_created_at, normalize_created_at};
use super::trace::{build_traces_payload, record_to_v2_span, record_to_v2_trace};

fn obj(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("valid fixture JSON")
}

fn val(text: &str) -> Json {
    serde_json::from_str(text).expect("valid fixture JSON")
}

fn object_field(fields: &IndexMap<String, Json>, key: &str) -> Json {
    fields[key].clone()
}

// Falsy messages (missing, None, []) falls back to a single synthetic user message rather than being preserved as an empty list.
fn check_message_fallback() {
    let fallback = vec![val(r#"{"role": "user", "content": "hello"}"#)];
    assert_eq!(galileo_input_messages(None, "hello"), fallback);
    assert_eq!(galileo_input_messages(Some(&Json::Null), "hello"), fallback);
    assert_eq!(
        galileo_input_messages(Some(&Json::List(vec![])), "hello"),
        fallback
    );
}

// Role is preserved and multi-part content is concatenated without a separator; a dict wrapper is unwrapped by its own "messages" key; an item lacking a role is skipped.
fn check_message_normalization() {
    let messages = val(r#"{"messages": [
            {"role": "system", "content": "be terse"},
            {"role": "assistant", "content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]},
            {"content": "no role, skipped"}
        ]}"#);
    assert_eq!(
        galileo_input_messages(Some(&messages), "fallback"),
        vec![
            val(r#"{"role": "system", "content": "be terse"}"#),
            val(r#"{"role": "assistant", "content": "ab"}"#),
        ],
    );
}

// A message whose role is present but falsy (null, here) is dropped entirely, not kept with an empty-string role.
fn check_message_falsy_role_dropped() {
    let messages = val(r#"[
            {"role": null, "content": "dropped"},
            {"role": "user", "content": "kept"}
        ]"#);
    assert_eq!(
        galileo_input_messages(Some(&messages), "fallback"),
        vec![val(r#"{"role": "user", "content": "kept"}"#)],
    );
}

// The reverse-order user/human search in input_text_from_messages picks the last user-role message over a later-in-reverse-order assistant message, proving the role check actually filters rather than taking whichever message comes first.
fn check_input_text_from_messages_role_priority() {
    let messages = val(r#"[
            {"role": "assistant", "content": "assistant message"},
            {"role": "user", "content": "user message"}
        ]"#);
    assert_eq!(input_text_from_messages(&messages), "user message");
}

// "human" is accepted by the same reverse-order search as "user", and a later assistant message in reverse order is still skipped in its favor.
fn check_input_text_from_messages_human_role() {
    let messages = val(r#"[
            {"role": "assistant", "content": "assistant message"},
            {"role": "human", "content": "human message"}
        ]"#);
    assert_eq!(input_text_from_messages(&messages), "human message");
}

// A tool message's search_results content counts source, title, block text and a present, non-null citations value, serialized back as JSON.
fn check_search_results_citations() {
    let message = obj(r#"{"role": "tool", "content": "", "search_results": [
            {"source": "wiki", "title": "Rust",
                "content": [{"type": "text", "text": " is a language"}],
                "citations": [{"url": "https://example.com"}]}
        ]}"#);
    assert_eq!(
        convert_content_list_to_str(&message),
        r#"wikiRust is a language[{"url":"https://example.com"}]"#
    );
}

// Missing fields read as zero, the total is computed from the per-token counts when absent, and cost is carried only when the record provided one.
fn check_token_metrics() {
    let record_no_total = obj(r#"{"num_input_tokens": 3, "num_output_tokens": 4}"#);
    let metrics = token_metrics_from_record(&record_no_total);
    assert_eq!(metrics.num_input_tokens, 3);
    assert_eq!(metrics.num_output_tokens, 4);
    assert_eq!(metrics.num_total_tokens, 7);
    assert_eq!(metrics.cost, None);

    let record_with_cost = obj(r#"{"num_total_tokens": 9, "cost": 0.5}"#);
    let metrics_with_cost = token_metrics_from_record(&record_with_cost);
    assert_eq!(metrics_with_cost.num_input_tokens, 0);
    assert_eq!(metrics_with_cost.num_total_tokens, 9);
    assert_eq!(metrics_with_cost.cost, Some(Decimal::new(5, 1)));

    // A total the record already gave, nonzero, is never recomputed, even when the per-token counts are also nonzero.
    let record_total_already_set = obj(r#"{"num_total_tokens": 5, "num_input_tokens": 2}"#);
    assert_eq!(
        token_metrics_from_record(&record_total_already_set).num_total_tokens,
        5
    );

    // The total is computed once either per-token count alone is nonzero, not only when both are.
    let record_input_only = obj(r#"{"num_input_tokens": 5}"#);
    assert_eq!(
        token_metrics_from_record(&record_input_only).num_total_tokens,
        5
    );
    let record_output_only = obj(r#"{"num_output_tokens": 5}"#);
    assert_eq!(
        token_metrics_from_record(&record_output_only).num_total_tokens,
        5
    );
}

// A small cost must serialize with its exact decimal digits, not a lossy f64 round-trip: a naive Decimal-to-f64-to-JSON-number path turns 0.00000123 into scientific notation (1.23e-6) instead of preserving the source text.
fn check_cost_precision_preserved() {
    let metrics = TokenMetrics {
        num_input_tokens: 0,
        num_output_tokens: 0,
        num_total_tokens: 0,
        cost: Some(Decimal::from_str_exact("0.00000123").expect("valid decimal literal")),
    };
    assert_eq!(
        object_field(&token_metrics_json(metrics), "cost"),
        val("0.00000123")
    );

    let record = LlmResponse {
        latency_ms: 0,
        status_code: 200,
        input_text: String::new(),
        output_text: String::new(),
        node_type: "llm".to_owned(),
        model: "gpt-4".to_owned(),
        num_input_tokens: 0,
        num_output_tokens: 0,
        num_total_tokens: 0,
        cost: Some(Decimal::from_str_exact("0.00000123").expect("valid decimal literal")),
        output_logprobs: None,
        created_at: "2024-01-01T00:00:00".to_owned(),
        tags: None,
        user_metadata: None,
    }
    .to_record(None);
    assert_eq!(object_field(&record, "cost"), val("0.00000123"));
}

// Each Json variant's own emptiness rule: zero and the empty number, string, list and object are falsy, everything else is truthy.
fn check_has_value() {
    assert!(!has_value(&Json::Null));
    assert!(!has_value(&val("false")));
    assert!(has_value(&val("true")));
    assert!(!has_value(&val("0")));
    assert!(has_value(&val("1")));
    assert!(!has_value(&val(r#""""#)));
    assert!(has_value(&val(r#""x""#)));
    assert!(!has_value(&val("[]")));
    assert!(has_value(&val("[1]")));
    assert!(!has_value(&val("{}")));
    assert!(has_value(&val(r#"{"k": 1}"#)));
}

fn sample_record() -> IndexMap<String, Json> {
    LlmResponse {
        latency_ms: 2,
        status_code: 200,
        input_text: "hi".to_owned(),
        output_text: "hello".to_owned(),
        node_type: "llm".to_owned(),
        model: "gpt-4".to_owned(),
        num_input_tokens: 1,
        num_output_tokens: 2,
        num_total_tokens: 3,
        cost: Some(Decimal::new(1, 2)),
        output_logprobs: None,
        created_at: "2024-01-01T00:00:00".to_owned(),
        tags: Some(vec!["prod".to_owned()]),
        user_metadata: None,
    }
    .to_record(None)
}

// The span links to its trace by the injected ids, carries the duration in nanoseconds and the aggregated metrics, and the trace wraps exactly that one span.
fn check_trace_envelope() {
    let record = sample_record();

    let span = record_to_v2_span(&record, "trace-1", "span-1");
    assert_eq!(object_field(&span, "id"), val(r#""span-1""#));
    assert_eq!(object_field(&span, "trace_id"), val(r#""trace-1""#));
    assert_eq!(object_field(&span, "parent_id"), val(r#""trace-1""#));
    assert_eq!(
        object_field(&span, "created_at"),
        val(r#""2024-01-01T00:00:00Z""#)
    );
    assert_eq!(
        object_field(&span, "output"),
        val(r#"{"role": "assistant", "content": "hello"}"#)
    );
    assert_eq!(
        object_field(&span, "input"),
        Json::List(vec![val(r#"{"role": "user", "content": "hi"}"#)]),
    );
    assert_eq!(object_field(&span, "tags"), val(r#"["prod"]"#));
    let Json::Object(span_metrics) = object_field(&span, "metrics") else {
        panic!("metrics is an object")
    };
    assert_eq!(object_field(&span_metrics, "duration_ns"), val("2000000"));
    assert_eq!(object_field(&span_metrics, "num_total_tokens"), val("3"));
    assert_eq!(object_field(&span_metrics, "cost"), val("0.01"));

    let trace = record_to_v2_trace(&record, "trace-1", "span-1");
    assert_eq!(object_field(&trace, "id"), val(r#""trace-1""#));
    assert_eq!(
        object_field(&trace, "spans"),
        Json::List(vec![Json::Object(span)])
    );
}

// `log_stream_id` is carried into the payload only when the caller supplied a non-empty one.
fn check_traces_payload() {
    let record = sample_record();
    let payload = build_traces_payload(
        &[record],
        &[("trace-1".to_owned(), "span-1".to_owned())],
        Some("stream-1"),
    );
    assert_eq!(
        object_field(&payload, "log_stream_id"),
        val(r#""stream-1""#)
    );
    let Json::List(traces) = object_field(&payload, "traces") else {
        panic!("traces is a list")
    };
    assert_eq!(traces.len(), 1);

    let payload_without_stream = build_traces_payload(&[], &[], None);
    assert!(!payload_without_stream.contains_key("log_stream_id"));
    let payload_with_empty_stream = build_traces_payload(&[], &[], Some(""));
    assert!(!payload_with_empty_stream.contains_key("log_stream_id"));
}

fn chat_message() -> Json {
    val(r#"{"content": null, "role": "assistant", "tool_calls": [
            {"function": {"arguments": "{}", "name": "f"}, "id": "call_1", "type": "function"}
        ], "function_call": null, "provider_specific_fields": null}"#)
}

// A chat response's message is carried through untouched (tool calls stay serialized, never decomposed), while the embedding and speech families return their literal marker strings.
fn check_content_selection() {
    let kwargs = GalileoKwargs {
        call_type: None,
        messages: Some(val(r#"[{"role": "user", "content": "hi"}]"#)),
        functions: None,
        tools: None,
        input: None,
    };
    let message = chat_message();
    let (input_text, output_text, _) = galileo_input_output_content(
        &kwargs,
        Some(&ResponseObj::Chat {
            message: Some(message.clone()),
        }),
        Level::Default,
        None,
    );
    assert_eq!(input_text, "hi");
    assert_eq!(output_text, serialize_galileo_output(Some(&message)));
    assert!(output_text.contains(
        r#""tool_calls": [{"function": {"arguments": "{}", "name": "f"}, "id": "call_1", "type": "function"}]"#
    ));

    let (_, embedding_output, _) =
        galileo_input_output_content(&kwargs, Some(&ResponseObj::Embedding), Level::Default, None);
    assert_eq!(embedding_output, "embedding-output");

    let (_, speech_output, _) =
        galileo_input_output_content(&kwargs, Some(&ResponseObj::Speech), Level::Default, None);
    assert_eq!(speech_output, "speech-output");
}

// call_type alone selects the embedding marker, even over a response the caller tagged as a different family, matching the call_type in (...) or isinstance(...) precedence.
fn check_embedding_call_type_precedence() {
    let embedding_kwargs = GalileoKwargs {
        call_type: Some("embedding".to_owned()),
        messages: None,
        functions: None,
        tools: None,
        input: None,
    };
    let (_, output, _) = galileo_input_output_content(
        &embedding_kwargs,
        Some(&ResponseObj::Chat {
            message: Some(chat_message()),
        }),
        Level::Default,
        None,
    );
    assert_eq!(output, "embedding-output");
}

// An error-level status message short-circuits before any response dispatch.
fn check_error_level_short_circuit() {
    let kwargs = GalileoKwargs::default();
    let (_, output, _) = galileo_input_output_content(&kwargs, None, Level::Error, Some("boom"));
    assert_eq!(output, "boom");
}

// litellm's get_content_from_model_response reconstructs the raw dict into a ModelResponse pydantic object, re-serializing tool_calls/function_call in that class's field order, not the caller's; a streaming chunk's choice has no "message" key, only "delta", and its text still reaches the output.
fn check_generic_dict_tool_call_reordering() {
    let kwargs = GalileoKwargs::default();

    let message_value = obj(
        r#"{"choices": [{"message": {"content": "dict response content",
            "tool_calls": [{"id": "call_1", "type": "function",
                "function": {"name": "foo", "arguments": "{}"}}]}}]}"#,
    );
    let (_, message_output, _) = galileo_input_output_content(
        &kwargs,
        Some(&ResponseObj::GenericDict {
            value: message_value,
        }),
        Level::Default,
        None,
    );
    assert_eq!(
        message_output,
        r#"dict response content{"function":{"arguments":"{}","name":"foo"},"id":"call_1","type":"function"}"#
    );

    // ModelResponse(**response) turns a dict choice into Choices(**choice), which always builds a Message even when the dict has no "message" key; a delta-only choice never reaches a StreamingChoices branch, so it contributes nothing.
    let delta_value = obj(r#"{"choices": [{"delta": {"content": "streamed text"}}]}"#);
    let (_, delta_output, _) = galileo_input_output_content(
        &kwargs,
        Some(&ResponseObj::GenericDict { value: delta_value }),
        Level::Default,
        None,
    );
    assert_eq!(delta_output, "");
}

// chat_completion_tool_call_from_dict dumps a custom tool call as {id, type: "custom", custom: {name, input}}, never the function shape. is_custom_tool_call_dict takes that branch for a "custom" type alone, and also for a non-null "custom" payload alone even when type says "function": the two conditions are independent, not both required.
fn check_custom_tool_call_dump() {
    let kwargs = GalileoKwargs::default();
    let value = obj(r#"{"choices": [{"message": {"tool_calls": [
            {"id": "call_1", "type": "custom", "custom": {"name": "exec", "input": "ls -la"}},
            {"id": "call_2", "type": "function", "function": {"name": "f", "arguments": "{}"},
             "custom": {"name": "exec2", "input": "pwd"}}
        ]}}]}"#);
    let (_, output, _) = galileo_input_output_content(
        &kwargs,
        Some(&ResponseObj::GenericDict { value }),
        Level::Default,
        None,
    );
    assert_eq!(
        output,
        concat!(
            r#"{"id":"call_1","type":"custom","custom":{"name":"exec","input":"ls -la"}}"#,
            r#"{"id":"call_2","type":"custom","custom":{"name":"exec2","input":"pwd"}}"#,
        )
    );
}

// Function.__init__ turns a dict `arguments` into its JSON text and reads a missing or explicitly null `arguments` as "", never as a nested object or a literal null.
fn check_function_arguments_normalization() {
    let kwargs = GalileoKwargs::default();
    let value = obj(r#"{"choices": [{"message": {"tool_calls": [
            {"id": "call_1", "type": "function", "function": {"name": "f", "arguments": {"x": 1}}},
            {"id": "call_2", "type": "function", "function": {"name": "g", "arguments": null}}
        ]}}]}"#);
    let (_, output, _) = galileo_input_output_content(
        &kwargs,
        Some(&ResponseObj::GenericDict { value }),
        Level::Default,
        None,
    );
    assert_eq!(
        output,
        concat!(
            r#"{"function":{"arguments":"{\"x\": 1}","name":"f"},"id":"call_1","type":"function"}"#,
            r#"{"function":{"arguments":"","name":"g"},"id":"call_2","type":"function"}"#,
        )
    );
}

// galileo.py only takes the realtime-list branch when call_type == "_arealtime" and the pass-through branch when call_type == "pass_through_endpoint"; any other call_type falls through (a list to the final fallback, a dict to the generic dict branch), ignoring the response's own shape.
fn check_call_type_mismatch_falls_through() {
    let realtime_kwargs = GalileoKwargs {
        call_type: Some("acompletion".to_owned()),
        messages: None,
        functions: None,
        tools: None,
        input: Some(val(r#"{"type": "response.create"}"#)),
    };
    let (_, realtime_output, _) = galileo_input_output_content(
        &realtime_kwargs,
        Some(&ResponseObj::RealtimeList {
            items: vec![val(r#"{"type": "response.done"}"#)],
        }),
        Level::Default,
        None,
    );
    assert_eq!(realtime_output, "");

    let pass_through_kwargs = GalileoKwargs {
        call_type: Some("completion".to_owned()),
        messages: None,
        functions: None,
        tools: None,
        input: None,
    };
    let mut pass_through_value = IndexMap::new();
    pass_through_value.insert("response".to_owned(), val(r#"{"should": "be-ignored"}"#));
    let (_, pass_through_output, _) = galileo_input_output_content(
        &pass_through_kwargs,
        Some(&ResponseObj::PassThroughDict {
            value: pass_through_value,
        }),
        Level::Default,
        None,
    );
    assert_eq!(pass_through_output, "");
}

// galileo.py:609 reads value["response"] verbatim for call_type == "pass_through_endpoint" and echoes the built prompt as messages_for_span, never the raw-dict content extraction the fallback branch uses.
fn check_pass_through_endpoint_content() {
    let kwargs = GalileoKwargs {
        call_type: Some("pass_through_endpoint".to_owned()),
        messages: None,
        functions: None,
        tools: None,
        input: None,
    };
    let mut value = IndexMap::new();
    value.insert(
        "response".to_owned(),
        val(r#"{"text": "passthrough output"}"#),
    );
    let (_, output, messages_for_span) = galileo_input_output_content(
        &kwargs,
        Some(&ResponseObj::PassThroughDict { value }),
        Level::Default,
        None,
    );
    assert_eq!(output, r#"{"text": "passthrough output"}"#);
    assert_eq!(messages_for_span, val(r#"{"messages": null}"#));
}

// _get_chat_content_for_galileo serializes message.json(), which is Message.model_dump(): content, role, tool_calls, function_call and provider_specific_fields always appear, nulls included, even for a raw provider message that supplied none of them.
fn check_chat_message_projection() {
    let kwargs = GalileoKwargs::default();
    let message = val(r#"{"role": "assistant", "content": "Hi there"}"#);
    let (_, output, _) = galileo_input_output_content(
        &kwargs,
        Some(&ResponseObj::Chat {
            message: Some(message),
        }),
        Level::Default,
        None,
    );
    assert_eq!(
        output,
        concat!(
            r#"{"content": "Hi there", "role": "assistant", "tool_calls": null, "#,
            r#""function_call": null, "provider_specific_fields": null}"#,
        )
    );
}

fn assert_chat_message_output(message: Json, expected: &str) {
    let kwargs = GalileoKwargs::default();
    let (_, output, _) = galileo_input_output_content(
        &kwargs,
        Some(&ResponseObj::Chat {
            message: Some(message),
        }),
        Level::Default,
        None,
    );
    assert_eq!(output, expected);
}

// An empty tool_calls list or a falsy function_call dict is present but folds back to null, same as an absent key; a truthy function_call takes its ordered dump instead.
fn check_message_model_dump_falsy_fields() {
    let cases = [
        (
            r#"{"role": "assistant", "content": null, "tool_calls": [], "function_call": {}}"#,
            concat!(
                r#"{"content": null, "role": "assistant", "tool_calls": null, "#,
                r#""function_call": null, "provider_specific_fields": null}"#,
            ),
        ),
        (
            r#"{"role": "assistant", "content": null, "function_call": {"name": "f", "arguments": "{}"}}"#,
            concat!(
                r#"{"content": null, "role": "assistant", "tool_calls": null, "#,
                r#""function_call": {"arguments": "{}", "name": "f"}, "provider_specific_fields": null}"#,
            ),
        ),
    ];
    for (input, expected) in cases {
        assert_chat_message_output(val(input), expected);
    }
}

// litellm.types.utils.Message.__init__ keeps annotations even when empty (an is-not-None check, not truthiness), keeps a present reasoning_content, carries a tool call's own extra "index" key ahead of function/id/type (ChatCompletionMessageToolCall.__init__ passes **params to super().__init__() before assigning self.function/id/type), and folds any key that isn't a declared Message field into the pydantic extra tail (here, a null "refusal"). A Chat response's messages_for_span still falls back to the (here absent) request messages, independent of what the response message carries.
fn check_message_model_dump_extended_fields() {
    let kwargs = GalileoKwargs::default();
    let message = val(
        r#"{"role": "assistant", "content": "Hi", "refusal": null, "annotations": [],
        "reasoning_content": "thinking...",
        "tool_calls": [{"index": 0, "id": "call_1", "type": "function",
            "function": {"name": "f", "arguments": "{}"}}]}"#,
    );
    let (_, output, messages_for_span) = galileo_input_output_content(
        &kwargs,
        Some(&ResponseObj::Chat {
            message: Some(message),
        }),
        Level::Default,
        None,
    );
    assert_eq!(
        output,
        concat!(
            r#"{"content": "Hi", "role": "assistant", "#,
            r#""tool_calls": [{"index": 0, "function": {"arguments": "{}", "name": "f"}, "id": "call_1", "type": "function"}], "#,
            r#""function_call": null, "reasoning_content": "thinking...", "#,
            r#""provider_specific_fields": null, "annotations": [], "refusal": null}"#,
        ),
    );
    assert_eq!(messages_for_span, Json::List(Vec::new()));
}

// Shared by both check_responses_api_*_projection tests: runs a raw Responses-API output list through galileo_input_output_content, asserts it parses to `expected`, and hands back the raw serialized text for the odd case that needs to see declared field order.
fn assert_responses_api_output(output: &str, expected: &str) -> String {
    let kwargs = GalileoKwargs::default();
    let (_, response_output, _) = galileo_input_output_content(
        &kwargs,
        Some(&ResponseObj::ResponsesApi {
            output: val(output),
        }),
        Level::Default,
        None,
    );
    let parsed: Json = serde_json::from_str(&response_output).expect("valid JSON output");
    assert_eq!(parsed, val(expected));
    response_output
}

// litellm.types.llms.openai.ResponsesAPIResponse.output parses each raw item into its matching openai.types.responses.ResponseOutputItem member (message, function_call or reasoning) before _serialize_galileo_output dumps them, filling every declared optional field with null and keeping any extra key the item or part carried; an item whose type isn't recognized or doesn't validate against its recognized type (including a reasoning summary part without a "summary_text" type) stays an unmodified dict.
fn check_responses_api_output_projection() {
    assert_responses_api_output(
        r#"[
            {"type": "message", "id": "msg_1", "role": "assistant", "status": "completed", "content": [{"type": "output_text", "text": "hi", "annotations": []}]},
            {"type": "message", "id": "msg_2", "role": "assistant", "status": "completed", "content": [{"type": "refusal", "refusal": "no", "foo": 1}]},
            {"type": "message", "id": "msg_3", "role": "assistant", "status": "completed", "content": [{"type": "something_else", "refusal": "no"}]},
            {"type": "function_call", "call_id": "call_1", "name": "f", "arguments": "{}"},
            {"type": "something_unknown", "foo": "bar"},
            {"type": "reasoning", "call_id": "call_2", "name": "g", "arguments": "{}"},
            {"type": "reasoning", "id": "rs_1", "summary": [], "foo": "bar"},
            {"type": "something_else", "id": "rs_2", "summary": []},
            {"type": "reasoning", "id": "rs_3", "summary": [{"text": "s"}]},
            {"type": "reasoning", "id": "rs_4", "summary": [], "status": "completed"},
            {"type": "reasoning", "id": "rs_5", "summary": [], "status": "bogus"}
        ]"#,
        r#"[
            {"id": "msg_1", "content": [{"annotations": [], "text": "hi", "type": "output_text", "logprobs": null}], "role": "assistant", "status": "completed", "type": "message", "phase": null},
            {"id": "msg_2", "content": [{"refusal": "no", "type": "refusal", "foo": 1}], "role": "assistant", "status": "completed", "type": "message", "phase": null},
            {"type": "message", "id": "msg_3", "role": "assistant", "status": "completed", "content": [{"type": "something_else", "refusal": "no"}]},
            {"arguments": "{}", "call_id": "call_1", "name": "f", "type": "function_call", "id": null, "caller": null, "namespace": null, "status": null},
            {"type": "something_unknown", "foo": "bar"},
            {"type": "reasoning", "call_id": "call_2", "name": "g", "arguments": "{}"},
            {"id": "rs_1", "summary": [], "type": "reasoning", "content": null, "encrypted_content": null, "status": null, "foo": "bar"},
            {"type": "something_else", "id": "rs_2", "summary": []},
            {"type": "reasoning", "id": "rs_3", "summary": [{"text": "s"}]},
            {"id": "rs_4", "summary": [], "type": "reasoning", "content": null, "encrypted_content": null, "status": "completed"},
            {"type": "reasoning", "id": "rs_5", "summary": [], "status": "bogus"}
        ]"#,
    );
}

// openai.types.responses.response_function_web_search.Action and ResponseCustomToolCall.Caller: an unrecognized or invalid action/caller invalidates the whole web_search_call or custom_tool_call item, which stays an unmodified dict.
fn check_responses_api_web_search_and_custom_tool_projection() {
    let response_output = assert_responses_api_output(
        r#"[
            {"type": "web_search_call", "id": "ws_1", "status": "completed", "action": {"type": "search", "query": "q"}},
            {"type": "web_search_call", "id": "ws_2", "status": "completed", "action": {"type": "bogus"}},
            {"type": "web_search_call", "id": "ws_3", "status": "completed", "action": {"type": "find_in_page", "pattern": "cat", "url": "http://example.com"}},
            {"type": "web_search_call", "id": "ws_4", "status": "completed", "action": {"type": "mystery", "pattern": "x", "url": "http://y"}},
            {"type": "web_search_call", "id": "ws_5", "status": "completed", "action": {"type": "search", "query": "q2", "sources": [{"type": "url", "url": "http://z"}]}},
            {"type": "web_search_call", "id": "ws_6", "status": "completed", "action": {"type": "search", "query": "q3", "sources": [{"type": "mystery", "url": "http://z"}]}},
            {"type": "custom_tool_call", "call_id": "c1", "input": "x", "name": "t", "caller": {"type": "program", "caller_id": "p1"}},
            {"type": "custom_tool_call", "call_id": "c2", "input": "x", "name": "t", "caller": {"type": "bogus"}}
        ]"#,
        r#"[
            {"id": "ws_1", "action": {"type": "search", "queries": null, "query": "q", "sources": null}, "status": "completed", "type": "web_search_call"},
            {"type": "web_search_call", "id": "ws_2", "status": "completed", "action": {"type": "bogus"}},
            {"id": "ws_3", "action": {"pattern": "cat", "type": "find_in_page", "url": "http://example.com"}, "status": "completed", "type": "web_search_call"},
            {"type": "web_search_call", "id": "ws_4", "status": "completed", "action": {"type": "mystery", "pattern": "x", "url": "http://y"}},
            {"id": "ws_5", "action": {"type": "search", "queries": null, "query": "q2", "sources": [{"type": "url", "url": "http://z"}]}, "status": "completed", "type": "web_search_call"},
            {"type": "web_search_call", "id": "ws_6", "status": "completed", "action": {"type": "search", "query": "q3", "sources": [{"type": "mystery", "url": "http://z"}]}},
            {"call_id": "c1", "input": "x", "name": "t", "type": "custom_tool_call", "id": null, "caller": {"caller_id": "p1", "type": "program"}, "namespace": null},
            {"type": "custom_tool_call", "call_id": "c2", "input": "x", "name": "t", "caller": {"type": "bogus"}}
        ]"#,
    );
    // A valid find_in_page action's projected and raw field sets are equal as parsed JSON; only the text shows typed_sequence's declared order.
    assert!(response_output.contains(
        r#""action": {"pattern": "cat", "type": "find_in_page", "url": "http://example.com"}"#
    ));
}

// Rerank uses its own langfuse-style prompt (just the messages, no functions/tools) rather than the shared build_prompt, and that prompt is echoed as the span's messages_for_span.
fn check_rerank_content() {
    let kwargs = GalileoKwargs {
        call_type: None,
        messages: Some(val(r#"[{"role": "user", "content": "rerank query"}]"#)),
        functions: None,
        tools: None,
        input: None,
    };
    let results = val(r#"[{"index": 0, "relevance_score": 0.9}]"#);
    let (_, output_text, messages_for_span) = galileo_input_output_content(
        &kwargs,
        Some(&ResponseObj::Rerank {
            results: results.clone(),
        }),
        Level::Default,
        None,
    );
    assert_eq!(
        messages_for_span,
        val(r#"{"messages": [{"role": "user", "content": "rerank query"}]}"#)
    );
    assert_eq!(output_text, serialize_galileo_output(Some(&results)));
}

// messages_for_span carries the caller's messages verbatim when they are present and truthy, and falls back to an empty list for a present-but-falsy value (not the falsy value itself).
fn check_messages_or_empty_guard() {
    let truthy_kwargs = GalileoKwargs {
        call_type: None,
        messages: Some(val(r#"[{"role": "user", "content": "hi"}]"#)),
        functions: None,
        tools: None,
        input: None,
    };
    let (_, _, truthy_messages_for_span) =
        galileo_input_output_content(&truthy_kwargs, None, Level::Default, None);
    assert_eq!(
        truthy_messages_for_span,
        truthy_kwargs.messages.expect("messages present")
    );

    let falsy_kwargs = GalileoKwargs {
        call_type: None,
        messages: Some(Json::Text(String::new())),
        functions: None,
        tools: None,
        input: None,
    };
    let (_, _, falsy_messages_for_span) =
        galileo_input_output_content(&falsy_kwargs, None, Level::Default, None);
    assert_eq!(falsy_messages_for_span, Json::List(Vec::new()));
}

#[test]
fn galileo_payload_construction() {
    check_message_fallback();
    check_message_normalization();
    check_message_falsy_role_dropped();
    check_input_text_from_messages_role_priority();
    check_input_text_from_messages_human_role();
    check_search_results_citations();
    check_token_metrics();
    check_cost_precision_preserved();
    check_has_value();
    check_trace_envelope();
    check_traces_payload();
    check_content_selection();
    check_embedding_call_type_precedence();
    check_error_level_short_circuit();
    check_generic_dict_tool_call_reordering();
    check_custom_tool_call_dump();
    check_function_arguments_normalization();
    check_call_type_mismatch_falls_through();
    check_pass_through_endpoint_content();
    check_chat_message_projection();
    check_message_model_dump_falsy_fields();
    check_message_model_dump_extended_fields();
    check_responses_api_output_projection();
    check_responses_api_web_search_and_custom_tool_projection();
    check_rerank_content();
    check_messages_or_empty_guard();
}

fn sample_naive_datetime() -> chrono::NaiveDateTime {
    let date = NaiveDate::from_ymd_opt(2024, 6, 1).expect("valid date");
    date.and_hms_opt(10, 0, 0).expect("valid time")
}

fn sample_offsets() -> (FixedOffset, FixedOffset) {
    let plus_three = FixedOffset::east_opt(3 * 3600).expect("valid offset");
    let minus_five = FixedOffset::west_opt(5 * 3600).expect("valid offset");
    (plus_three, minus_five)
}

// A naive datetime is assumed to be local time (the caller's own injected offset) and converted to UTC; an already-aware datetime converts from its own offset directly, ignoring the caller's local offset.
fn check_timestamp_offset_handling() {
    let naive = sample_naive_datetime();
    let (plus_three, minus_five) = sample_offsets();
    assert_eq!(
        format_created_at(RawTimestamp::Naive(naive), plus_three),
        "2024-06-01T07:00:00Z"
    );
    assert_eq!(
        format_created_at(RawTimestamp::Naive(naive), minus_five),
        "2024-06-01T15:00:00Z"
    );

    let aware = plus_three
        .from_local_datetime(&naive)
        .single()
        .expect("a fixed offset has no ambiguous local time");
    assert_eq!(
        format_created_at(RawTimestamp::Aware(aware), minus_five),
        "2024-06-01T07:00:00Z"
    );
}

// Normalization appends Z only when no timezone suffix (Z or +-HH:MM) is present already.
fn check_normalize_created_at_suffixes() {
    assert_eq!(
        normalize_created_at("2024-06-01T07:00:00"),
        "2024-06-01T07:00:00Z"
    );
    assert_eq!(
        normalize_created_at("2024-06-01T07:00:00Z"),
        "2024-06-01T07:00:00Z"
    );
    assert_eq!(
        normalize_created_at("2024-06-01T07:00:00+02:00"),
        "2024-06-01T07:00:00+02:00"
    );
    assert_eq!(
        normalize_created_at("2024-06-01T07:00:00+0200"),
        "2024-06-01T07:00:00+0200"
    );
    assert_eq!(normalize_created_at(""), "");
}

#[test]
fn galileo_timestamp_handling() {
    check_timestamp_offset_handling();
    check_normalize_created_at_suffixes();
}
