#![cfg(test)]

use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tracing::field::Field as EventField;
use tracing::subscriber::with_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;
use uuid::Uuid;

use super::decode_event_line;
use super::error::ResponsesChatStreamError;
use super::records::{ChatChunk, ChunkFactory};
use super::stream::{ParsedChatLine, ResponsesChatStream, get_model_response_iterator};
use super::text::repr_value;
use super::translate::translate_event;
use crate::api::api_boundaries::ApiLimits;

const FIXED_ID: u128 = 0x0123_4567_89ab_4def_8123_4567_89ab_cdef;
const FIXED_CLOCK: i64 = 1_700_000_000;
const GENERATED_ID: &str = "chatcmpl-01234567-89ab-4def-8123-456789abcdef";

fn fixed_identifier() -> Uuid {
    Uuid::from_u128(FIXED_ID)
}

fn fixed_clock() -> i64 {
    FIXED_CLOCK
}

fn factory() -> ChunkFactory {
    ChunkFactory {
        identifier: fixed_identifier,
        clock: fixed_clock,
    }
}

fn stream() -> ResponsesChatStream {
    get_model_response_iterator(Some(false), fixed_identifier, fixed_clock)
}

fn event(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn rendered(chunk: &ChatChunk) -> Value {
    serde_json::to_value(chunk).unwrap()
}

fn translated(chunk: &Value) -> Result<Value, ResponsesChatStreamError> {
    translate_event(chunk, None, factory()).map(|chunk| rendered(&chunk))
}

// LiteLLM's Delta carries these five keys on every chunk, null where unset
fn empty_delta() -> Value {
    json!({"content": "", "role": null, "function_call": null, "tool_calls": null, "audio": null})
}

fn choice(delta: Value, finish_reason: Value, index: Value) -> Value {
    json!({"index": index, "delta": delta, "finish_reason": finish_reason, "logprobs": null})
}

fn chunk_of(id: &str, choices: Value) -> Value {
    json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": FIXED_CLOCK,
        "choices": choices,
        "provider_specific_fields": null,
    })
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<String>>>);

impl<Inner: Subscriber> Layer<Inner> for Captured {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = String::new();
        event.record(&mut |field: &EventField, value: &dyn fmt::Debug| {
            message = format!("{value:?}");
        });
        self.0.lock().unwrap().push(message);
    }
}

fn capture<Output, Call>(call: Call) -> (Output, Vec<String>)
where
    Call: FnOnce() -> Output,
{
    let captured = Captured::default();
    let subscriber = registry().with(captured.clone());
    let output = with_default(subscriber, call);
    let lines = captured.0.lock().unwrap().clone();
    (output, lines)
}

fn padding_value() -> Value {
    json!({
        "text": "",
        "tool_use": null,
        "is_finished": false,
        "finish_reason": "",
        "usage": null,
    })
}

#[rstest::rstest]
#[case::empty("")]
#[case::event_line("event: response.created")]
fn event_line_padding_covers_empty_and_event_lines(#[case] line: &str) {
    let decoded = decode_event_line(line.as_bytes(), &ApiLimits::default()).unwrap();
    assert_eq!(
        decoded,
        super::DecodedEventLine::Padding(super::records::PaddingChunk {
            text: String::new(),
            tool_use: None,
            is_finished: false,
            finish_reason: String::new(),
            usage: None,
        })
    );
}

#[test]
fn event_line_removes_everything_through_the_first_data_prefix() {
    let parsed =
        decode_event_line(br#"prefix data:{"type":"unknown"}"#, &ApiLimits::default()).unwrap();
    assert_eq!(
        parsed,
        super::DecodedEventLine::Event(event(r#"{"type":"unknown"}"#))
    );
    let parsed = decode_event_line(
        br#"data: {"type": "response.output_text.delta", "delta": "hi"}"#,
        &ApiLimits::default(),
    )
    .unwrap();
    assert_eq!(
        parsed,
        super::DecodedEventLine::Event(event(
            r#"{"type": "response.output_text.delta", "delta": "hi"}"#
        ))
    );
}

#[test]
fn event_line_rejects_invalid_json_with_the_json_decode_error_text() {
    let failure = decode_event_line(b"{", &ApiLimits::default()).unwrap_err();
    assert_eq!(
        failure.to_string(),
        "Expecting property name enclosed in double quotes: line 1 column 2 (char 1)"
    );
    let done = decode_event_line(b"[DONE]", &ApiLimits::default()).unwrap_err();
    assert_eq!(
        done.to_string(),
        "Expecting value: line 1 column 2 (char 1)"
    );
    let prefixed = decode_event_line(b"data: [DONE]", &ApiLimits::default()).unwrap_err();
    assert_eq!(
        prefixed.to_string(),
        "Expecting value: line 1 column 3 (char 2)"
    );
    let spaces = decode_event_line(b"   ", &ApiLimits::default()).unwrap_err();
    assert_eq!(
        spaces.to_string(),
        "Expecting value: line 1 column 4 (char 3)"
    );
    let array = decode_event_line(b"[DONE, 1]", &ApiLimits::default()).unwrap_err();
    assert_eq!(
        array.to_string(),
        "Expecting value: line 1 column 2 (char 1)"
    );
}

#[test]
fn event_line_checks_the_event_byte_limit() {
    let limits = ApiLimits {
        max_json_bytes: 16,
        max_event_bytes: 4,
        max_items: 16,
    };
    let failure = decode_event_line(b"data: {}", &limits).unwrap_err();
    assert!(matches!(failure, ResponsesChatStreamError::Limit(_)));
}

#[test]
fn stateless_translation_rejects_empty_and_non_object_chunks() {
    let empty = translate_event(&json!({}), None, factory()).unwrap_err();
    assert_eq!(empty.to_string(), "Chat provider: Empty parsed_chunk");
    let list = translate_event(&json!([1]), None, factory()).unwrap_err();
    assert_eq!(
        list.to_string(),
        "Chat provider: Invalid chunk type <class 'list'>"
    );
    let text = translate_event(&json!("x"), None, factory()).unwrap_err();
    assert_eq!(
        text.to_string(),
        "Chat provider: Invalid chunk type <class 'str'>"
    );
    // the errors carry no HTTP fields, so a caller answers with LiteLLM's plain ValueError
    assert!(matches!(
        list,
        ResponsesChatStreamError::InvalidChunkType { kind: "list" }
    ));
}

#[test]
fn a_typeless_payload_without_choices_is_not_a_chat_chunk() {
    let chunk = translated(&json!({"model": "m"})).unwrap();
    assert_eq!(chunk["id"], json!(GENERATED_ID));
    assert!(chunk.get("model").is_none());
}

#[rstest::rstest]
#[case::integer("5", "int")]
#[case::float("1.5", "float")]
#[case::boolean("true", "bool")]
fn stateless_translation_names_the_rejected_chunk_type(#[case] source: &str, #[case] kind: &str) {
    let failure = translate_event(&event(source), None, factory()).unwrap_err();
    assert_eq!(
        failure.to_string(),
        format!("Chat provider: Invalid chunk type <class '{kind}'>")
    );
}

#[rstest::rstest]
#[case::zero("0")]
#[case::null("null")]
#[case::empty_list("[]")]
#[case::empty_text("\"\"")]
fn stateless_translation_rejects_falsy_chunks(#[case] falsy: &str) {
    let failure = translate_event(&event(falsy), None, factory()).unwrap_err();
    assert_eq!(failure.to_string(), "Chat provider: Empty parsed_chunk");
}

#[test]
fn stateless_translation_passes_a_chat_chunk_through() {
    let chunk = translated(&json!({
        "object": "chat.completion.chunk",
        "id": "cid",
        "created": 5,
        "model": "m",
        "extra_key": 1,
        "choices": [{"index": 1, "delta": {"content": "c"}, "finish_reason": "max_tokens"}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3},
    }))
    .unwrap();
    let expected = json!({
        "id": "cid",
        "object": "chat.completion.chunk",
        "created": 5,
        "model": "m",
        "extra_key": 1,
        "choices": [{
            "index": 1,
            "delta": {"content": "c", "role": null, "function_call": null, "tool_calls": null, "audio": null},
            "finish_reason": "length",
            "logprobs": null,
        }],
        "usage": {
            "prompt_tokens": 1,
            "completion_tokens": 2,
            "total_tokens": 3,
            "prompt_tokens_details": null,
            "completion_tokens_details": null,
        },
        "provider_specific_fields": null,
    });
    assert_eq!(chunk, expected);
}

#[test]
fn stateless_translation_passes_a_missing_type_with_choices_through() {
    let chunk =
        translated(&json!({"choices": [{"index": 0, "delta": {"content": "hi"}}]})).unwrap();
    assert_eq!(chunk["id"], json!(GENERATED_ID));
    assert_eq!(chunk["choices"][0]["delta"]["content"], json!("hi"));
    let bare = translated(&json!({"choices": []})).unwrap();
    assert_eq!(bare["choices"][0]["index"], json!(0));
    assert_eq!(bare["choices"][0]["delta"]["content"], json!(null));
}

#[test]
fn stateless_translation_maps_an_unhandled_type_to_an_empty_chunk() {
    let chunk = translated(&json!({"type": "response.failed"})).unwrap();
    assert_eq!(
        chunk,
        chunk_of(
            GENERATED_ID,
            json!([choice(empty_delta(), json!(null), json!(0))])
        )
    );
}

#[test]
fn stateless_translation_creates_an_id_from_the_created_event() {
    let chunk = translated(&json!({
        "type": "response.created",
        "response": {"id": "resp_a"},
    }))
    .unwrap();
    assert_eq!(
        chunk,
        chunk_of(
            "resp_a",
            json!([choice(empty_delta(), json!(null), json!(0))])
        )
    );
}

#[test]
fn stateless_translation_generates_an_id_without_a_created_response() {
    let ids = [
        json!({"type": "response.created"}),
        json!({"type": "response.created", "response": null}),
        json!({"type": "response.created", "response": {}}),
        json!({"type": "response.created", "response": {"id": null}}),
    ]
    .map(|source| translated(&source).unwrap()["id"].clone());
    assert_eq!(ids, [json!(GENERATED_ID); 4]);
}

#[test]
fn stateless_translation_carries_tool_calls_on_a_new_item() {
    let chunk = translated(&json!({
        "type": "response.output_item.added",
        "output_index": 2,
        "item": {"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "f", "arguments": "{}"},
    }))
    .unwrap();
    let expected_call = json!({
        "id": "call_1",
        "index": 2,
        "type": "function",
        "function": {"name": "f", "arguments": "{}"},
    });
    let delta = json!({
        "content": null,
        "role": null,
        "function_call": null,
        "tool_calls": [expected_call],
        "audio": null,
    });
    assert_eq!(
        chunk,
        chunk_of(GENERATED_ID, json!([choice(delta, json!(null), json!(0))]))
    );
}

#[test]
fn stateless_translation_defaults_a_missing_output_index_to_zero() {
    let chunk = translated(&json!({
        "type": "response.output_item.added",
        "item": {"type": "custom_tool_call", "call_id": "c1", "input": "raw"},
    }))
    .unwrap();
    let call = &chunk["choices"][0]["delta"]["tool_calls"][0];
    assert_eq!(call["index"], json!(0));
    assert_eq!(call["id"], json!("c1"));
    assert_eq!(call["function"]["name"], json!("custom_tool"));
    assert_eq!(call["function"]["arguments"], json!("raw"));
}

#[test]
fn stateless_translation_turns_an_empty_converted_name_into_null() {
    let chunk = translated(&json!({
        "type": "response.output_item.added",
        "output_index": 0,
        "item": {"type": "function_call", "call_id": "c", "name": "", "arguments": ""},
        "arguments": "top",
    }))
    .unwrap();
    let call = &chunk["choices"][0]["delta"]["tool_calls"][0];
    assert_eq!(call["function"]["name"], json!(null));
    assert_eq!(call["function"]["arguments"], json!("top"));
}

#[test]
fn stateless_translation_rejects_an_empty_argument_delta() {
    let failure = translated(&json!({
        "type": "response.function_call_arguments.delta",
        "delta": "",
    }))
    .unwrap_err();
    assert_eq!(
        failure.to_string(),
        "Chat provider: Invalid function argument delta {'type': 'response.function_call_arguments.delta', 'delta': ''}"
    );
    let missing = translated(&json!({
        "type": "response.custom_tool_call_input.delta",
        "delta": null,
    }))
    .unwrap_err();
    assert_eq!(
        missing.to_string(),
        "Chat provider: Invalid function argument delta {'type': 'response.custom_tool_call_input.delta', 'delta': None}"
    );
}

#[test]
fn stateless_translation_carries_an_argument_delta_at_the_raw_index() {
    let chunk = translated(&json!({
        "type": "response.custom_tool_call_input.delta",
        "output_index": 5,
        "delta": "x",
    }))
    .unwrap();
    let call = &chunk["choices"][0]["delta"]["tool_calls"][0];
    assert_eq!(
        call,
        &json!({"id": null, "index": 5, "type": "function", "function": {"name": null, "arguments": "x"}})
    );
}

#[test]
fn stateless_translation_rejects_a_missing_text_delta() {
    let failure = translated(&json!({"type": "response.output_text.delta"})).unwrap_err();
    assert_eq!(
        failure.to_string(),
        "Chat provider: Invalid text delta {'type': 'response.output_text.delta'}"
    );
}

#[test]
fn stateless_translation_keeps_an_empty_text_delta() {
    let chunk = translated(&json!({"type": "response.output_text.delta", "delta": ""})).unwrap();
    assert_eq!(chunk["choices"][0]["delta"]["content"], json!(""));
    let text = translated(&json!({"type": "response.output_text.delta", "delta": "hi"})).unwrap();
    assert_eq!(text["choices"][0]["delta"]["content"], json!("hi"));
}

#[test]
fn stateless_translation_uses_the_summary_index_for_reasoning() {
    let chunk = translated(&json!({
        "type": "response.reasoning_summary_text.delta",
        "summary_index": 3,
        "delta": "x",
    }))
    .unwrap();
    assert_eq!(chunk["choices"][0]["index"], json!(3));
    assert_eq!(
        chunk["choices"][0]["delta"]["reasoning_content"],
        json!("x")
    );
    let absent = translated(&json!({
        "type": "response.reasoning_summary_text.delta",
        "delta": "x",
    }))
    .unwrap();
    assert_eq!(absent["choices"][0]["index"], json!(null));
}

#[test]
fn stateless_translation_makes_an_empty_reasoning_delta_an_unhandled_event() {
    let chunk = translated(&json!({
        "type": "response.reasoning_summary_text.delta",
        "summary_index": 1,
        "delta": "",
    }))
    .unwrap();
    assert_eq!(
        chunk,
        chunk_of(
            GENERATED_ID,
            json!([choice(empty_delta(), json!(null), json!(0))])
        )
    );
}

#[test]
fn stateless_translation_done_item_is_not_terminal() {
    let item = json!({"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "f", "arguments": "{}"});
    let complete = translated(&json!({
        "type": "response.output_item.done",
        "output_index": 2,
        "item": item,
    }))
    .unwrap();
    let call = json!({
        "id": "call_1",
        "index": 2,
        "type": "function",
        "function": {"name": "f", "arguments": "{}"},
    });
    let delta = json!({
        "content": null,
        "role": null,
        "function_call": null,
        "tool_calls": [call],
        "audio": null,
    });
    assert_eq!(
        complete,
        chunk_of(GENERATED_ID, json!([choice(delta, json!(null), json!(0))]))
    );

    // the stateful stream already sent the call, so its done event carries an empty delta and never a finish reason
    let mut iterator = stream();
    let stateful = iterator
        .chunk_parser(&json!({
            "type": "response.output_item.done",
            "output_index": 2,
            "item": item,
        }))
        .unwrap();
    let empty = json!({
        "content": null,
        "role": null,
        "function_call": null,
        "tool_calls": null,
        "audio": null,
    });
    assert_eq!(
        rendered(&stateful),
        chunk_of(GENERATED_ID, json!([choice(empty, json!(null), json!(0))]))
    );
}

#[test]
fn stateless_translation_done_message_gives_empty_content() {
    let chunk = translated(&json!({
        "type": "response.output_item.done",
        "item": {"type": "message"},
    }))
    .unwrap();
    assert_eq!(chunk["choices"][0]["delta"]["content"], json!(""));
    assert_eq!(chunk["choices"][0]["finish_reason"], json!(null));
}

#[test]
fn stateless_translation_sequential_indices_need_a_map() {
    let added = |index: i64| {
        json!({
            "type": "response.output_item.added",
            "output_index": index,
            "item": {"type": "function_call", "id": "f", "call_id": "c", "name": "n", "arguments": "{}"},
        })
    };
    let mut iterator = stream();
    let indices = [2_i64, 5, 2].map(|index| {
        let chunk = iterator.chunk_parser(&added(index)).unwrap();
        chunk.choices[0].delta.tool_calls.as_ref().unwrap()[0].index
    });
    assert_eq!(indices, [Some(0), Some(1), Some(0)]);
    let raw = [2_i64, 5, 2].map(|index| {
        translated(&added(index)).unwrap()["choices"][0]["delta"]["tool_calls"][0]["index"].clone()
    });
    assert_eq!(raw, [json!(2), json!(5), json!(2)]);
}

#[test]
fn stateless_translation_accepts_a_shared_tool_call_map() {
    let mut map = super::translate::ToolCallIndices::new();
    let added = |index: i64| {
        json!({
            "type": "response.output_item.added",
            "output_index": index,
            "item": {"type": "function_call", "id": "f", "call_id": "c", "name": "n", "arguments": "{}"},
        })
    };
    let first = translate_event(&added(7), Some(&mut map), factory())
        .map(|chunk| rendered(&chunk))
        .unwrap();
    assert_eq!(
        first["choices"][0]["delta"]["tool_calls"][0]["index"],
        json!(0)
    );
    assert_eq!(map.get(&7), Some(&0));
}

#[test]
fn stateless_translation_finishes_on_completed_output() {
    let plain = translated(&json!({
        "type": "response.completed",
        "response": {"id": "resp_a", "output": [{"type": "message"}]},
    }))
    .unwrap();
    assert_eq!(plain["choices"][0]["finish_reason"], json!("stop"));
    assert_eq!(plain["usage"], json!(null));

    let calls = translated(&json!({
        "type": "response.completed",
        "response": {"id": "resp_a", "output": [{"type": "custom_tool_call"}]},
    }))
    .unwrap();
    assert_eq!(calls["choices"][0]["finish_reason"], json!("tool_calls"));

    let patch = translated(&json!({
        "type": "response.completed",
        "response": {"id": "resp_a", "output": [{"type": "apply_patch_call"}]},
    }))
    .unwrap();
    assert_eq!(patch["choices"][0]["finish_reason"], json!("stop"));
}

#[test]
fn stateless_translation_finishes_an_incomplete_response_by_its_reason() {
    let filtered = translated(&json!({
        "type": "response.incomplete",
        "response": {"incomplete_details": {"reason": "content_filter"}},
    }))
    .unwrap();
    assert_eq!(
        filtered["choices"][0]["finish_reason"],
        json!("content_filter")
    );
    let lengths = [
        json!({"type": "response.incomplete"}),
        json!({"type": "response.incomplete", "response": {"incomplete_details": {"reason": "max_output_tokens"}}}),
        json!({"type": "response.incomplete", "response": {"incomplete_details": {"reason": null}}}),
    ]
    .map(|source| translated(&source).unwrap()["choices"][0]["finish_reason"].clone());
    assert_eq!(lengths, [json!("length"); 3]);
}

#[test]
fn stateless_translation_carries_usage_and_reasoning_items_on_completion() {
    let chunk = translated(&json!({
        "type": "response.completed",
        "response": {
            "id": "resp_a",
            "output": [
                {"type": "reasoning", "id": "rs_1", "summary": [{"type": "summary_text", "text": "t"}]},
                {"type": "message"},
                7,
            ],
            "usage": {"input_tokens": 1, "output_tokens": 2, "total_tokens": 3},
        },
    }))
    .unwrap();
    assert_eq!(chunk["choices"][0]["finish_reason"], json!("stop"));
    assert_eq!(
        chunk["choices"][0]["delta"]["reasoning_items"],
        json!([{"type": "reasoning", "id": "rs_1", "encrypted_content": null, "summary": [{"type": "summary_text", "text": "t"}]}])
    );
    assert_eq!(
        chunk["usage"],
        json!({
            "prompt_tokens": 1,
            "completion_tokens": 2,
            "total_tokens": 3,
            "prompt_tokens_details": null,
            "completion_tokens_details": null,
        })
    );
}

#[test]
fn usage_details_follow_the_shared_projection() {
    let chunk = translated(&json!({
        "type": "response.completed",
        "response": {
            "usage": {
                "input_tokens": 2,
                "output_tokens": 5,
                "total_tokens": 7,
                "input_tokens_details": {"cached_tokens": 1},
                "output_tokens_details": {"reasoning_tokens": 3},
            },
        },
    }))
    .unwrap();
    assert_eq!(
        chunk["usage"]["prompt_tokens_details"]["cached_tokens"],
        json!(1)
    );
    assert_eq!(
        chunk["usage"]["completion_tokens_details"]["reasoning_tokens"],
        json!(3)
    );
}

#[test]
fn stateless_translation_keeps_the_provider_metadata() {
    let chunk = translated(&json!({
        "type": "response.completed",
        "response": {"id": "resp_a", "custom_key": 5, "service_tier": "scale", "output": []},
    }))
    .unwrap();
    assert_eq!(
        chunk["provider_specific_fields"],
        json!({"custom_key": 5, "service_tier": "scale"})
    );
    assert_eq!(chunk["service_tier"], json!("scale"));
}

#[test]
fn stateless_translation_keeps_an_empty_terminal_tier() {
    let chunk = translated(&json!({
        "type": "response.completed",
        "response": {"incomplete_details": null, "service_tier": ""},
    }))
    .unwrap();
    assert_eq!(chunk["service_tier"], json!(""));
}

#[test]
fn stream_identity_keeps_the_created_id_on_every_chunk() {
    let mut iterator = stream();
    let created = iterator
        .chunk_parser(&json!({"type": "response.created", "response": {"id": "resp_a"}}))
        .unwrap();
    assert_eq!(created.id, "resp_a");
    let delta = iterator
        .chunk_parser(&json!({"type": "response.output_text.delta", "delta": "hi"}))
        .unwrap();
    assert_eq!(delta.id, "resp_a");
    assert_eq!(delta.choices[0].delta.content.as_deref(), Some("hi"));
    let terminal = iterator
        .chunk_parser(&json!({"type": "response.completed", "response": {"id": "resp_b"}}))
        .unwrap();
    assert_eq!(terminal.id, "resp_a");
}

#[test]
fn stream_identity_keeps_the_first_generated_id() {
    let mut iterator = stream();
    let first = iterator
        .chunk_parser(&json!({"type": "response.output_text.delta", "delta": "a"}))
        .unwrap();
    assert_eq!(first.id, GENERATED_ID);
    let created = iterator
        .chunk_parser(&json!({"type": "response.created", "response": {"id": "resp_a"}}))
        .unwrap();
    assert_eq!(created.id, GENERATED_ID);
}

#[test]
fn stream_identity_is_never_shared_between_iterators() {
    let mut first = stream();
    let mut second = stream();
    let chunk = json!({"type": "response.created", "response": {"id": "resp_a"}});
    assert_eq!(first.chunk_parser(&chunk).unwrap().id, "resp_a");
    let other = second
        .chunk_parser(&json!({"type": "response.created", "response": {"id": "resp_b"}}))
        .unwrap();
    assert_eq!(other.id, "resp_b");
}

#[test]
fn stream_tier_is_remembered_and_overridden() {
    let mut iterator = stream();
    let created = iterator
        .chunk_parser(&json!({"type": "response.created", "response": {"id": "resp_a", "service_tier": "scale"}}))
        .unwrap();
    assert_eq!(created.service_tier.as_deref(), Some("scale"));
    let carried = iterator
        .chunk_parser(&json!({"type": "response.output_text.delta", "delta": "b"}))
        .unwrap();
    assert_eq!(carried.service_tier.as_deref(), Some("scale"));
    let reset = iterator
        .chunk_parser(
            &json!({"type": "response.created", "response": {"id": "resp_a", "service_tier": ""}}),
        )
        .unwrap();
    assert_eq!(reset.service_tier.as_deref(), Some("scale"));
    let terminal = iterator
        .chunk_parser(&json!({"type": "response.completed", "response": {"id": "resp_a", "service_tier": ""}}))
        .unwrap();
    assert_eq!(terminal.service_tier.as_deref(), Some(""));
}

#[test]
fn stream_tier_ignores_a_non_string_tier() {
    let mut iterator = stream();
    iterator
        .chunk_parser(
            &json!({"type": "response.created", "response": {"id": "resp_a", "service_tier": 7}}),
        )
        .unwrap();
    let carried = iterator
        .chunk_parser(&json!({"type": "response.output_text.delta", "delta": "b"}))
        .unwrap();
    assert_eq!(carried.service_tier, None);
}

#[test]
fn stream_logs_each_translation_step() {
    let mut iterator = stream();
    let (created, lines) = capture(|| {
        iterator
            .chunk_parser(&json!({"type": "response.created", "response": {"id": "resp_a"}}))
            .unwrap()
    });
    assert_eq!(
        lines,
        vec![
            "Chat provider: transform_streaming_response called with chunk: {'type': 'response.created', 'response': {'id': 'resp_a'}}".to_owned(),
            "Chat provider: Processing event type: response.created".to_owned(),
            "Chat provider: response.created -> {'type': 'response.created', 'response': {'id': 'resp_a'}}".to_owned(),
        ]
    );
    assert_eq!(created.id, "resp_a");

    let (_, passthrough) = capture(|| {
        iterator
            .chunk_parser(&json!({"object": "chat.completion.chunk", "choices": [{"index": 0}]}))
            .unwrap()
    });
    assert_eq!(passthrough.len(), 1);
    assert!(
        passthrough[0]
            .starts_with("Chat provider: transform_streaming_response called with chunk: ")
    );

    let (_, unhandled) = capture(|| {
        iterator
            .chunk_parser(&json!({"type": "response.failed"}))
            .unwrap()
    });
    assert_eq!(
        unhandled.last().unwrap(),
        "Chat provider: Unhandled event type 'response.failed', creating empty chunk"
    );

    let (_, conversation) = capture(|| {
        iterator
            .chunk_parser(&json!({"type": "conversation.item.created"}))
            .unwrap()
    });
    assert!(
        conversation.contains(
            &"Chat provider: Processing event type: conversation.item.created".to_owned()
        )
    );
}

#[test]
fn stream_parse_line_feeds_decoded_events_and_padding() {
    let mut iterator = stream();
    let padding = iterator
        .parse_line(b"event: response.created", &ApiLimits::default())
        .unwrap();
    match padding {
        ParsedChatLine::Padding(chunk) => {
            assert_eq!(serde_json::to_value(chunk).unwrap(), padding_value());
        }
        ParsedChatLine::Chunk(_) => panic!("an event line is no chunk"),
    }
    let line = br#"data: {"type": "response.output_text.delta", "delta": "hi"}"#;
    match iterator.parse_line(line, &ApiLimits::default()).unwrap() {
        ParsedChatLine::Chunk(chunk) => {
            assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("hi"));
        }
        ParsedChatLine::Padding(_) => panic!("a data line is a chunk"),
    }
}

#[test]
fn stream_parse_line_reports_a_malformed_line() {
    let mut iterator = stream();
    let failure = iterator
        .parse_line(b"{", &ApiLimits::default())
        .unwrap_err();
    assert_eq!(
        failure.to_string(),
        "Expecting property name enclosed in double quotes: line 1 column 2 (char 1)"
    );
}

#[test]
fn repr_matches_python_text() {
    assert_eq!(repr_value(&json!(null)), "None");
    assert_eq!(repr_value(&json!(true)), "True");
    assert_eq!(repr_value(&json!(false)), "False");
    assert_eq!(repr_value(&json!(1.5)), "1.5");
    assert_eq!(repr_value(&json!("it's")), "'it\\'s'");
    assert_eq!(repr_value(&json!("a\\b\nc\rd\te")), "'a\\\\b\\nc\\rd\\te'");
    assert_eq!(repr_value(&json!(["a", 1])), "['a', 1]");
    assert_eq!(repr_value(&json!({"a": {"b": "c"}})), "{'a': {'b': 'c'}}");
}

#[test]
fn get_model_response_iterator_keeps_its_json_mode() {
    let iterator = get_model_response_iterator(Some(true), fixed_identifier, fixed_clock);
    assert_eq!(iterator.json_mode, Some(true));
    assert_eq!(iterator.chat_completion_id, None);
    assert_eq!(iterator.served_service_tier, None);
    assert!(iterator.tool_call_index_map.is_empty());
    let defaulted = get_model_response_iterator(None, fixed_identifier, fixed_clock);
    assert_eq!(defaulted.json_mode, None);
}

#[test]
fn replays_corpus() {
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/responses_chat_event_line");
    let mut iterator = stream();
    let replayed = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            let _replayed = iterator.parse_line(&bytes, &ApiLimits::default());
        })
        .count();
    assert!(replayed >= 6);
}
