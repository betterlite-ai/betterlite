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
use super::metrics::token_metrics_from_record;
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

// litellm's get_content_from_model_response reconstructs the raw dict into a ModelResponse pydantic object, re-serializing tool_calls/function_call in that class's field order, not the caller's.
fn check_generic_dict_tool_call_reordering() {
    let kwargs = GalileoKwargs::default();
    let value = obj(
        r#"{"choices": [{"message": {"content": "dict response content",
            "tool_calls": [{"id": "call_1", "type": "function",
                "function": {"name": "foo", "arguments": "{}"}}]}}]}"#,
    );
    let (_, output, _) = galileo_input_output_content(
        &kwargs,
        Some(&ResponseObj::GenericDict { value }),
        Level::Default,
        None,
    );
    assert_eq!(
        output,
        r#"dict response content{"function":{"arguments":"{}","name":"foo"},"id":"call_1","type":"function"}"#
    );
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
    check_search_results_citations();
    check_token_metrics();
    check_trace_envelope();
    check_traces_payload();
    check_content_selection();
    check_embedding_call_type_precedence();
    check_error_level_short_circuit();
    check_generic_dict_tool_call_reordering();
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
