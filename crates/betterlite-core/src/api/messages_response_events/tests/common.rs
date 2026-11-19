use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponseApiUsage;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;

use crate::api::api_boundaries::ApiLimits;
use crate::api::messages_options::INCOMPLETE_STREAM_MESSAGE;
use crate::api::messages_options::refusal::OutputItem;
use crate::api::messages_response_events::MessageStreamState;
use crate::api::messages_response_events::failure::{
    anthropic_error_chunk, failure_status_and_message,
};
use crate::api::messages_response_events::input::{
    EventAttributes, ParsedEvent, ResponseAttribute, ResponseAttributes,
};
use crate::api::responses_stream_errors::failure::FailureCause;

pub(super) const CREATED: &str = r#"{"type":"response.created"}"#;
pub(super) const MESSAGE_START: &str = r#"[{"type":"message_start","message":{"id":"msg_test","type":"message","role":"assistant","content":[],"model":"model_a","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}]"#;
pub(super) const MESSAGE_ITEM: &str =
    r#"{"type":"response.output_item.added","item":{"type":"message","id":"item_a"}}"#;
pub(super) const FAILED_DEFAULT: &str =
    r#"[{"type":"error","error":{"type":"api_error","message":"Response API in-stream error"}}]"#;
pub(super) const FAILED_LOG_DEFAULT: &str = "AnthropicResponsesStreamWrapper: upstream Responses stream for model_a failed (500): Response API in-stream error";

pub(super) fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("valid fixture")
}

pub(super) fn mapping(source: &str) -> IndexMap<String, Json> {
    match json(source) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("a fixture event is a JSON object")
        }
    }
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn state() -> MessageStreamState {
    MessageStreamState::new(
        "msg_test".to_owned(),
        "model_a".to_owned(),
        None,
        ApiLimits::default(),
    )
}

pub(super) fn process(state: &mut MessageStreamState, event: &IndexMap<String, Json>) {
    state
        .process_event(ParsedEvent {
            attributes: EventAttributes::default(),
            mapping: Some(event),
        })
        .expect("the event processes");
}

pub(super) fn drained(state: &mut MessageStreamState) -> Json {
    let mut chunks = Vec::new();
    while let Some(chunk) = state.chunk_queue.pop_front() {
        chunks.push(chunk);
    }
    Json::List(chunks)
}

pub(super) fn discard(state: &mut MessageStreamState) {
    while state.chunk_queue.pop_front().is_some() {}
}

pub(super) fn assert_mapped(state: &MessageStreamState, item_id: &str, block_idx: i64) {
    assert_eq!(
        state
            .snapshot()
            .item_id_to_block_index
            .get(item_id)
            .copied(),
        Some(block_idx)
    );
}

pub(super) fn failure(source: &str) -> FailureCause {
    serde_json::from_str(source).expect("valid failure")
}

pub(super) fn usage(source: &str) -> ResponseApiUsage {
    serde_json::from_str(source).expect("valid usage")
}

pub(super) fn completed_usage() -> ResponseApiUsage {
    usage(
        r#"{"input_tokens":10,"input_tokens_details":{"cached_tokens":3},"output_tokens":2,"total_tokens":12}"#,
    )
}

pub(super) fn projected_usage() -> Json {
    json(r#"{"input_tokens":7,"output_tokens":2,"cache_read_input_tokens":3}"#)
}

pub(super) fn zero_usage() -> Json {
    json(r#"{"input_tokens":0,"output_tokens":0}"#)
}

pub(super) fn terminal_chunks(stop_reason: &str, explanation: Option<&str>, usage: Json) -> Json {
    let mut delta = IndexMap::new();
    delta.insert("stop_reason".to_owned(), text(stop_reason));
    delta.insert("stop_sequence".to_owned(), Json::Null);
    if let Some(label) = explanation {
        let mut details = IndexMap::new();
        details.insert("type".to_owned(), text("refusal"));
        details.insert("category".to_owned(), Json::Null);
        details.insert("explanation".to_owned(), text(label));
        delta.insert("stop_details".to_owned(), Json::Object(details));
    }
    let mut message_delta = IndexMap::new();
    message_delta.insert("type".to_owned(), text("message_delta"));
    message_delta.insert("delta".to_owned(), Json::Object(delta));
    message_delta.insert("usage".to_owned(), usage);
    let mut message_stop = IndexMap::new();
    message_stop.insert("type".to_owned(), text("message_stop"));
    Json::List(vec![
        Json::Object(message_delta),
        Json::Object(message_stop),
    ])
}

pub(super) fn completed(
    state: &mut MessageStreamState,
    event_type: &str,
    output: &[OutputItem<'_>],
    response_status: Option<&str>,
    usage: Option<&ResponseApiUsage>,
) {
    let kind = text(event_type);
    let attributes = EventAttributes {
        event_type: Some(&kind),
        item: None,
        item_id: None,
        delta: None,
        summary_index: None,
        response: Some(ResponseAttribute::Object(ResponseAttributes {
            output,
            status: response_status,
            usage,
            error: None,
        })),
    };
    state
        .process_event(ParsedEvent {
            attributes,
            mapping: None,
        })
        .expect("the event processes");
}

// A valid status needs an int in 400..=599 or a decimal string in that range.
pub(super) fn failure_status_matrix() {
    let statuses: &[(&str, u16)] = &[
        ("429", 429),
        ("\"\u{0664}\u{0662}\u{0669}\"", 429),
        ("true", 500),
        ("429.0", 500),
        ("\"+429\"", 500),
        ("\" 429 \"", 500),
        ("399", 500),
        ("600", 500),
        ("\"abc\"", 500),
        ("null", 500),
        ("400", 400),
        ("599", 599),
    ];
    for &(raw, expected) in statuses {
        let cause = failure(&format!(
            r#"{{"kind":"plain","text":"m","status_code":{raw}}}"#
        ));
        assert_eq!(
            failure_status_and_message(&cause),
            (expected, "m".to_owned()),
            "status {raw}"
        );
    }
}

// A message needs a nonempty string; any other value falls back to the failure text.
pub(super) fn failure_message_matrix() {
    let messages: &[(&str, &str)] = &[
        ("\"busy\"", "busy"),
        ("429", "boom"),
        ("3.5", "boom"),
        ("null", "boom"),
        ("\"\"", "boom"),
        ("\"x\"", "x"),
    ];
    for &(raw, expected) in messages {
        let cause = failure(&format!(
            r#"{{"kind":"plain","text":"boom","message":{raw}}}"#
        ));
        assert_eq!(
            failure_status_and_message(&cause),
            (500, expected.to_owned()),
            "message {raw}"
        );
    }
}

pub(super) fn failure_chains() {
    let chain = failure(
        "{\"kind\":\"mid_stream_fallback\",\"text\":\"outer\",\"status_code\":502,\"original_exception\":{\"kind\":\"mid_stream_fallback\",\"text\":\"inner\",\"original_exception\":{\"kind\":\"plain\",\"text\":\"boom\",\"status_code\":\"\u{0664}\u{0662}\u{0669}\",\"message\":\"busy\"}}}",
    );
    assert_eq!(failure_status_and_message(&chain), (429, "busy".to_owned()));
    let unlinked = failure(r#"{"kind":"mid_stream_fallback","text":"outer","status_code":503}"#);
    assert_eq!(
        failure_status_and_message(&unlinked),
        (503, "outer".to_owned())
    );
    let incomplete = failure(r#"{"kind":"plain","text":""}"#);
    assert_eq!(
        failure_status_and_message(&incomplete),
        (500, INCOMPLETE_STREAM_MESSAGE.to_owned())
    );
}

pub(super) fn mapped_error_types() {
    let limits = ApiLimits::default();
    let types = [
        (400, "invalid_request_error"),
        (401, "authentication_error"),
        (403, "permission_error"),
        (404, "not_found_error"),
        (413, "request_too_large"),
        (429, "rate_limit_error"),
        (500, "api_error"),
        (529, "overloaded_error"),
        (418, "api_error"),
    ];
    for (status, error_type) in types {
        let chunk = anthropic_error_chunk(status, "plain", None, limits).expect("the error maps");
        let expected = json(&format!(
            r#"{{"type":"error","error":{{"type":"{error_type}","message":"plain"}}}}"#
        ));
        assert_eq!(chunk, expected, "status {status}");
    }
}

pub(super) fn passthrough_error_messages() {
    let limits = ApiLimits::default();
    let passthrough = anthropic_error_chunk(
        500,
        r#"{"type":"error","error":{"type":"custom_type","message":"from upstream"}}"#,
        None,
        limits,
    )
    .expect("the error maps");
    assert_eq!(
        passthrough,
        json(r#"{"type":"error","error":{"type":"custom_type","message":"from upstream"}}"#)
    );
    let detail =
        anthropic_error_chunk(500, r#"{"detail":"d"}"#, None, limits).expect("the error maps");
    assert_eq!(
        detail,
        json(r#"{"type":"error","error":{"type":"api_error","message":"{\"detail\":\"d\"}"}}"#)
    );
}

// MINIMUM_CUSTOM_KEY_LENGTH is 16, so a sk- token needs 13 characters after the prefix to scrub.
pub(super) fn failure_redaction() {
    let limits = ApiLimits::default();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the built-in patterns compile");
    let scrubbed = anthropic_error_chunk(500, "sk-1234567890abcdef boom", Some(&redactor), limits)
        .expect("the error maps");
    assert_eq!(
        scrubbed,
        json(r#"{"type":"error","error":{"type":"api_error","message":"REDACTED boom"}}"#)
    );
    let original = anthropic_error_chunk(500, "sk-1234567890abcdef boom", None, limits)
        .expect("the error maps");
    assert_eq!(
        original,
        json(
            r#"{"type":"error","error":{"type":"api_error","message":"sk-1234567890abcdef boom"}}"#
        )
    );
    let short = anthropic_error_chunk(500, "sk-SECRET boom", Some(&redactor), limits)
        .expect("the error maps");
    assert_eq!(
        short,
        json(r#"{"type":"error","error":{"type":"api_error","message":"sk-SECRET boom"}}"#)
    );
}

pub(super) fn failed_event_redaction() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the built-in patterns compile");
    let mut state = MessageStreamState::new(
        "msg_test".to_owned(),
        "model_a".to_owned(),
        Some(Arc::new(redactor)),
        ApiLimits::default(),
    );
    let failed = mapping(
        r#"{"type":"response.failed","response":{"error":{"type":"rate_limit_error","message":"sk-1234567890abcdef busy"}}}"#,
    );
    process(&mut state, &failed);
    let snapshot = state.snapshot();
    assert_eq!(
        Json::List(snapshot.chunks),
        json(r#"[{"type":"error","error":{"type":"rate_limit_error","message":"REDACTED busy"}}]"#)
    );
    assert_eq!(
        snapshot.logs.first().map(|log| log.message.as_str()),
        Some(
            "AnthropicResponsesStreamWrapper: upstream Responses stream for model_a failed (429): sk-1234567890abcdef busy"
        )
    );
}

pub(super) fn message_start_once() {
    let created = mapping(CREATED);
    let mut state = state();
    process(&mut state, &created);
    process(&mut state, &created);
    assert_eq!(drained(&mut state), json(MESSAGE_START));
    assert!(state.snapshot().sent_message_start.0);
}

pub(super) fn silent_events() {
    let mut state = state();
    process(
        &mut state,
        &mapping(r#"{"type":"response.output_text.done"}"#),
    );
    process(&mut state, &mapping("{}"));
    process(
        &mut state,
        &mapping(r#"{"type":"response.output_item.added"}"#),
    );
    assert_eq!(drained(&mut state), json("[]"));
    assert!(!state.snapshot().sent_message_start.0);
}

pub(super) fn open_message_and_tool() {
    let function_item = mapping(
        r#"{"type":"response.output_item.added","item":{"type":"function_call","id":"item_tool","call_id":"call_a","name":"lookup"}}"#,
    );
    let mut state = state();
    process(&mut state, &mapping(MESSAGE_ITEM));
    process(&mut state, &function_item);
    assert_eq!(
        drained(&mut state),
        json(
            r#"[{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"call_a","name":"lookup","input":{}}}]"#
        )
    );
    let snapshot = state.snapshot();
    assert_eq!(
        snapshot.item_id_to_block_index.get("item_a").copied(),
        Some(0)
    );
    assert_eq!(
        snapshot.pending_tool_ids.get("item_tool").cloned(),
        Some(text("call_a"))
    );
    assert_eq!(snapshot.current_block_index, 1);
}

pub(super) fn reopen_and_empty_id() {
    let message_item = mapping(MESSAGE_ITEM);
    let defaults =
        mapping(r#"{"type":"response.output_item.added","item":{"type":"function_call"}}"#);
    let empty_id =
        mapping(r#"{"type":"response.output_item.added","item":{"type":"message","id":""}}"#);
    let mut state = state();
    process(&mut state, &message_item);
    discard(&mut state);
    process(&mut state, &defaults);
    assert_eq!(
        drained(&mut state),
        json(
            r#"[{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"","name":"","input":{}}}]"#
        )
    );
    process(&mut state, &message_item);
    assert_eq!(
        drained(&mut state),
        json(
            r#"[{"type":"content_block_start","index":2,"content_block":{"type":"text","text":""}}]"#
        )
    );
    assert_mapped(&state, "item_a", 2);
    process(&mut state, &empty_id);
    assert_eq!(
        drained(&mut state),
        json(
            r#"[{"type":"content_block_start","index":3,"content_block":{"type":"text","text":""}}]"#
        )
    );
    assert!(!state.snapshot().item_id_to_block_index.contains_key(""));
}

pub(super) fn non_block_items() {
    let reasoning = mapping(
        r#"{"type":"response.output_item.added","item":{"type":"reasoning","id":"reason_a"}}"#,
    );
    let search = mapping(
        r#"{"type":"response.output_item.added","item":{"type":"web_search_call","id":"s"}}"#,
    );
    let mut state = state();
    process(&mut state, &reasoning);
    process(&mut state, &search);
    assert_eq!(drained(&mut state), json("[]"));
    assert!(state.snapshot().item_id_to_block_index.is_empty());
}

pub(super) fn text_deltas() {
    let delta = mapping(r#"{"type":"response.output_text.delta","item_id":"item_a","delta":"hi"}"#);
    let empty = mapping(r#"{"type":"response.output_text.delta","item_id":"item_a","delta":""}"#);
    let mut state = state();
    process(&mut state, &mapping(MESSAGE_ITEM));
    discard(&mut state);
    process(&mut state, &delta);
    assert_eq!(
        drained(&mut state),
        json(
            r#"[{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}]"#
        )
    );
    process(&mut state, &empty);
    assert_eq!(
        drained(&mut state),
        json(
            r#"[{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":""}}]"#
        )
    );
}

pub(super) fn text_before_block() {
    let before =
        mapping(r#"{"type":"response.output_text.delta","item_id":"item_a","delta":"first"}"#);
    let missing = mapping(r#"{"type":"response.output_text.delta","item_id":"item_a"}"#);
    let mut state = state();
    process(&mut state, &before);
    assert_eq!(
        drained(&mut state),
        json(
            r#"[{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"first"}}]"#
        )
    );
    assert_mapped(&state, "item_a", 0);
    process(&mut state, &missing);
    assert_eq!(
        drained(&mut state),
        json(
            r#"[{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":""}}]"#
        )
    );
}

pub(super) fn refusal_deltas() {
    let refusal = mapping(r#"{"type":"response.refusal.delta","item_id":"reason_a","delta":"no"}"#);
    let second = mapping(r#"{"type":"response.refusal.delta","item_id":"reason_a","delta":"pe"}"#);
    let empty = mapping(r#"{"type":"response.refusal.delta","item_id":"reason_a","delta":""}"#);
    let number = mapping(r#"{"type":"response.refusal.delta","item_id":"reason_a","delta":7}"#);
    let mut state = state();
    process(&mut state, &refusal);
    assert_eq!(
        drained(&mut state),
        json(
            r#"[{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"no"}}]"#
        )
    );
    assert_eq!(state.snapshot().refusal_text, "no");
    process(&mut state, &second);
    assert_eq!(state.snapshot().refusal_text, "nope");
    process(&mut state, &empty);
    process(&mut state, &number);
    assert_eq!(
        drained(&mut state),
        json(
            r#"[{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"pe"}}]"#
        )
    );
}

pub(super) fn attributed_falsey_fallbacks() {
    let falsey_delta = text("");
    let falsey_id = text("");
    let attributes = EventAttributes {
        event_type: None,
        item: None,
        item_id: Some(&falsey_id),
        delta: Some(&falsey_delta),
        summary_index: None,
        response: None,
    };
    let fields =
        mapping(r#"{"type":"response.output_text.delta","item_id":"item_a","delta":"fallback"}"#);
    let mut state = state();
    state
        .process_event(ParsedEvent {
            attributes,
            mapping: Some(&fields),
        })
        .expect("the event processes");
    assert_eq!(
        drained(&mut state),
        json(
            r#"[{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"fallback"}}]"#
        )
    );
}

pub(super) fn attributed_truthy_delta() {
    let fields =
        mapping(r#"{"type":"response.output_text.delta","item_id":"item_a","delta":"fallback"}"#);
    let truthy_delta = text("attr");
    let truthy_attributes = EventAttributes {
        event_type: None,
        item: None,
        item_id: None,
        delta: Some(&truthy_delta),
        summary_index: None,
        response: None,
    };
    let mut state = state();
    state
        .process_event(ParsedEvent {
            attributes: truthy_attributes,
            mapping: Some(&fields),
        })
        .expect("the event processes");
    assert_eq!(
        drained(&mut state),
        json(
            r#"[{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"attr"}}]"#
        )
    );
}

// A null attribute type falls back to the dict's type; a falsey "" one wins and no message starts.
pub(super) fn attributed_event_type_fallback() {
    let created = mapping(CREATED);
    let null_type = Json::Null;
    let empty_type = text("");
    for (event_type, starts) in [(&null_type, true), (&empty_type, false)] {
        let attributes = EventAttributes {
            event_type: Some(event_type),
            item: None,
            item_id: None,
            delta: None,
            summary_index: None,
            response: None,
        };
        let mut state = state();
        state
            .process_event(ParsedEvent {
                attributes,
                mapping: Some(&created),
            })
            .expect("the event processes");
        assert_eq!(
            state.snapshot().sent_message_start.0,
            starts,
            "{event_type:?}"
        );
        let expected = if starts {
            json(MESSAGE_START)
        } else {
            json("[]")
        };
        assert_eq!(drained(&mut state), expected, "{event_type:?}");
    }
}
