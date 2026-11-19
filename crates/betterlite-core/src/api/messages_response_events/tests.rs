#![cfg(test)]

use betterlite_models::decisions::Json;

use crate::api::messages_options::refusal::OutputItem;
use crate::api::responses_stream_primitives::error_fields::StreamErrorAttributes;

use super::error::MessagesResponseEventError;
use super::input::{
    ErrorAttribute, EventAttributes, ParsedEvent, ResponseAttribute, ResponseAttributes,
};

mod common;
use common::{
    FAILED_DEFAULT, FAILED_LOG_DEFAULT, MESSAGE_ITEM, attributed_empty_type, attributed_fallbacks,
    attributed_null_type, completed, completed_usage, discard, drained, failed_event_redaction,
    failure, failure_chains, failure_message_matrix, failure_redaction, failure_status_matrix,
    json, mapped_error_types, mapping, message_start_once, non_block_items, open_message_and_tool,
    passthrough_error_messages, process, process_attribute, projected_usage, refusal_deltas,
    reopen_and_empty_id, silent_events, state, terminal_chunks, text, text_before_block,
    text_deltas, usage, zero_usage,
};

#[test]
fn failure_projection() {
    failure_status_matrix();
    failure_message_matrix();
    failure_chains();
    mapped_error_types();
    passthrough_error_messages();
    failure_redaction();
    failed_event_redaction();
    exception_path_failure();
}

// The exception path of __anext__ projects the failure chain into the terminal error chunk without a wrapper log.
fn exception_path_failure() {
    let cause = failure(
        r#"{"kind":"mid_stream_fallback","text":"outer","status_code":502,"original_exception":{"kind":"plain","text":"boom","status_code":429,"message":"busy"}}"#,
    );
    let mut state = state();
    state.fail_with(&cause).expect("the failure maps");
    let snapshot = state.snapshot();
    assert!(snapshot.stream_failed);
    assert_eq!(
        Json::List(snapshot.chunks),
        json(r#"[{"type":"error","error":{"type":"rate_limit_error","message":"busy"}}]"#)
    );
    assert!(snapshot.logs.is_empty());
}

#[test]
fn message_start_and_item_opening() {
    message_start_once();
    silent_events();
    open_message_and_tool();
    reopen_and_empty_id();
    non_block_items();
    item_attribute_precedes_dict();
}

// getattr(event, "item") is truthy here, so the dict item is never read and no block opens.
fn item_attribute_precedes_dict() {
    let attribute_item = json("5");
    let attributes = EventAttributes {
        item: Some(&attribute_item),
        ..EventAttributes::default()
    };
    let fields =
        mapping(r#"{"type":"response.output_item.added","item":{"type":"message","id":"item_a"}}"#);
    let mut state = state();
    process_attributed(&mut state, attributes, &fields);
    assert_eq!(drained(&mut state), json("[]"));
    assert!(state.snapshot().item_id_to_block_index.is_empty());
}

#[test]
fn text_and_refusal_deltas() {
    text_deltas();
    text_before_block();
    refusal_deltas();
    attributed_fallbacks();
    attributed_null_type();
    attributed_empty_type();
    two_blocks_target_the_first();
    strange_item_ids();
}

// item_a stays mapped at 0 while the current block is the tool block at 1.
fn two_blocks_target_the_first() {
    let function_item = mapping(
        r#"{"type":"response.output_item.added","item":{"type":"function_call","id":"item_tool","call_id":"call_a","name":"lookup"}}"#,
    );
    let delta = mapping(r#"{"type":"response.output_text.delta","item_id":"item_a","delta":"hi"}"#);
    let done =
        mapping(r#"{"type":"response.output_item.done","item":{"type":"message","id":"item_a"}}"#);
    let mut state = state();
    process(&mut state, &mapping(MESSAGE_ITEM));
    process(&mut state, &function_item);
    discard(&mut state);
    process(&mut state, &delta);
    assert_eq!(
        drained(&mut state),
        json(
            r#"[{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}]"#
        )
    );
    process(&mut state, &done);
    assert_eq!(
        drained(&mut state),
        json(r#"[{"type":"content_block_stop","index":0}]"#)
    );
}

// A truthy nonstring item id selects no entry, while an empty list or object is falsey and keeps the current block.
fn strange_item_ids() {
    for raw in ["5", "[]", "{}"] {
        let mut state = state();
        process(&mut state, &mapping(MESSAGE_ITEM));
        discard(&mut state);
        let event = mapping(&format!(
            r#"{{"type":"response.output_text.delta","item_id":{raw},"delta":"d"}}"#
        ));
        process(&mut state, &event);
        let expected = if raw == "5" {
            json(
                r#"[{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}},{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"d"}}]"#,
            )
        } else {
            json(
                r#"[{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"d"}}]"#,
            )
        };
        assert_eq!(drained(&mut state), expected, "item_id {raw}");
        assert_eq!(
            state
                .snapshot()
                .item_id_to_block_index
                .get("item_a")
                .copied(),
            Some(0),
            "item_id {raw}"
        );
    }
}

fn summary_delta_opens_block() {
    let delta = mapping(
        r#"{"type":"response.reasoning_summary_text.delta","item_id":"reason_a","delta":"think"}"#,
    );
    let mut state = state();
    process(&mut state, &delta);
    assert_eq!(
        drained(&mut state),
        json(
            r#"[{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}},{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"think"}}]"#
        )
    );
    assert_eq!(
        state
            .snapshot()
            .item_id_to_block_index
            .get("reason_a")
            .copied(),
        Some(0)
    );
}

fn summary_delta_needs_text() {
    let empty =
        mapping(r#"{"type":"response.reasoning_summary_text.delta","item_id":"ghost","delta":""}"#);
    let mut state = state();
    process(&mut state, &empty);
    assert_eq!(drained(&mut state), json("[]"));
    assert!(state.snapshot().item_id_to_block_index.is_empty());
    let into_text =
        mapping(r#"{"type":"response.reasoning_summary_text.delta","item_id":"","delta":""}"#);
    process(&mut state, &mapping(MESSAGE_ITEM));
    discard(&mut state);
    process(&mut state, &into_text);
    assert_eq!(
        drained(&mut state),
        json(
            r#"[{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":""}}]"#
        )
    );
}

fn summary_part_indexes() {
    let delta = mapping(
        r#"{"type":"response.reasoning_summary_text.delta","item_id":"reason_a","delta":"think"}"#,
    );
    let mut state = state();
    process(&mut state, &delta);
    discard(&mut state);
    for index in ["-1", "-0", "true", "false", "0", "1.5", "\"1\""] {
        let part = mapping(&format!(
            r#"{{"type":"response.reasoning_summary_part.added","item_id":"reason_a","summary_index":{index}}}"#
        ));
        process(&mut state, &part);
        let expected = if index == "-1" || index == "true" {
            json(
                r#"[{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"\n\n"}}]"#,
            )
        } else {
            json("[]")
        };
        assert_eq!(drained(&mut state), expected, "summary_index {index}");
    }
    let no_index =
        mapping(r#"{"type":"response.reasoning_summary_part.added","item_id":"reason_a"}"#);
    process(&mut state, &no_index);
    assert_eq!(drained(&mut state), json("[]"));
    let unmapped = mapping(
        r#"{"type":"response.reasoning_summary_part.added","item_id":"other","summary_index":2}"#,
    );
    process(&mut state, &unmapped);
    assert_eq!(drained(&mut state), json("[]"));
}

// Only a string item id reaches the map; a missing or nonstring one never falls back to the current block.
fn summary_part_without_string_id() {
    let mut state = state();
    process(
        &mut state,
        &mapping(
            r#"{"type":"response.reasoning_summary_text.delta","item_id":"reason_a","delta":"think"}"#,
        ),
    );
    discard(&mut state);
    for source in [
        r#"{"type":"response.reasoning_summary_part.added","summary_index":1}"#,
        r#"{"type":"response.reasoning_summary_part.added","item_id":5,"summary_index":1}"#,
    ] {
        process(&mut state, &mapping(source));
        assert_eq!(drained(&mut state), json("[]"), "{source}");
    }
}

#[test]
fn reasoning_summary_boundaries() {
    summary_delta_opens_block();
    summary_delta_needs_text();
    summary_part_indexes();
    summary_part_without_string_id();
}

fn reasoning_signature_mapped() {
    let delta = mapping(
        r#"{"type":"response.reasoning_summary_text.delta","item_id":"reason_a","delta":"think"}"#,
    );
    let done = mapping(
        r#"{"type":"response.output_item.done","item":{"type":"reasoning","id":"reason_a","encrypted_content":"cipher"}}"#,
    );
    let mut state = state();
    process(&mut state, &delta);
    discard(&mut state);
    process(&mut state, &done);
    assert_eq!(
        drained(&mut state),
        json(
            r#"[{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"litellm_encrypted_reasoning:cipher"}},{"type":"content_block_stop","index":0}]"#
        )
    );
}

fn reasoning_redacted_block() {
    let done = mapping(
        r#"{"type":"response.output_item.done","item":{"type":"reasoning","id":"reason_a","encrypted_content":"cipher"}}"#,
    );
    let mut state = state();
    process(&mut state, &done);
    assert_eq!(
        drained(&mut state),
        json(
            r#"[{"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":"litellm_encrypted_reasoning:cipher"}},{"type":"content_block_stop","index":0}]"#
        )
    );
    assert_eq!(
        state
            .snapshot()
            .item_id_to_block_index
            .get("reason_a")
            .copied(),
        Some(0)
    );
}

fn reasoning_without_signature() {
    let no_signature = mapping(
        r#"{"type":"response.output_item.done","item":{"type":"reasoning","id":"reason_a"}}"#,
    );
    let empty_signature = mapping(
        r#"{"type":"response.output_item.done","item":{"type":"reasoning","id":"reason_a","encrypted_content":""}}"#,
    );
    let null_signature = mapping(
        r#"{"type":"response.output_item.done","item":{"type":"reasoning","id":"reason_a","encrypted_content":null}}"#,
    );
    let mut state = state();
    process(&mut state, &no_signature);
    process(&mut state, &empty_signature);
    process(&mut state, &null_signature);
    assert_eq!(drained(&mut state), json("[]"));
}

fn reasoning_stops_mapped_block() {
    let mapped = mapping(
        r#"{"type":"response.output_item.done","item":{"type":"reasoning","id":"item_a"}}"#,
    );
    let done_message =
        mapping(r#"{"type":"response.output_item.done","item":{"type":"message","id":"item_a"}}"#);
    let mut state = state();
    process(&mut state, &mapping(MESSAGE_ITEM));
    discard(&mut state);
    process(&mut state, &mapped);
    assert_eq!(
        drained(&mut state),
        json(r#"[{"type":"content_block_stop","index":0}]"#)
    );
    process(&mut state, &done_message);
    assert_eq!(
        drained(&mut state),
        json(r#"[{"type":"content_block_stop","index":0}]"#)
    );
}

fn reasoning_stops_current_block() {
    let missing_id = mapping(r#"{"type":"response.output_item.done","item":{"type":"message"}}"#);
    let mut state = state();
    process(&mut state, &mapping(MESSAGE_ITEM));
    discard(&mut state);
    process(&mut state, &missing_id);
    assert_eq!(
        drained(&mut state),
        json(r#"[{"type":"content_block_stop","index":0}]"#)
    );
}

#[test]
fn reasoning_item_closure() {
    reasoning_signature_mapped();
    reasoning_redacted_block();
    reasoning_without_signature();
    reasoning_stops_mapped_block();
    reasoning_stops_current_block();
}

fn arguments_before_block() {
    let before = mapping(
        r#"{"type":"response.function_call_arguments.delta","item_id":"tool_a","delta":"{\"x\":"}"#,
    );
    let mut state = state();
    process(&mut state, &before);
    assert_eq!(
        drained(&mut state),
        json(
            r#"[{"type":"content_block_delta","index":-1,"delta":{"type":"input_json_delta","partial_json":"{\"x\":"}}]"#
        )
    );
    assert!(state.snapshot().item_id_to_block_index.is_empty());
}

fn arguments_deltas() {
    let function_item = mapping(
        r#"{"type":"response.output_item.added","item":{"type":"function_call","id":"item_tool","call_id":"call_a","name":"lookup"}}"#,
    );
    let mapped = mapping(
        r#"{"type":"response.function_call_arguments.delta","item_id":"item_tool","delta":"1}"}"#,
    );
    let empty = mapping(
        r#"{"type":"response.function_call_arguments.delta","item_id":"item_tool","delta":""}"#,
    );
    let absent = mapping(r#"{"type":"response.function_call_arguments.delta","delta":"tail"}"#);
    let empty_id =
        mapping(r#"{"type":"response.function_call_arguments.delta","item_id":"","delta":"e"}"#);
    let mut state = state();
    process(&mut state, &function_item);
    discard(&mut state);
    process(&mut state, &mapped);
    process(&mut state, &empty);
    process(&mut state, &absent);
    process(&mut state, &empty_id);
    assert_eq!(
        drained(&mut state),
        json(
            r#"[{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"1}"}},{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":""}},{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"tail"}},{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"e"}}]"#
        )
    );
}

fn done_stops_mapped_block() {
    let function_item = mapping(
        r#"{"type":"response.output_item.added","item":{"type":"function_call","id":"item_tool","call_id":"call_a","name":"lookup"}}"#,
    );
    let done = mapping(
        r#"{"type":"response.output_item.done","item":{"type":"function_call","id":"item_tool"}}"#,
    );
    let ghost = mapping(
        r#"{"type":"response.output_item.done","item":{"type":"function_call","id":"ghost"}}"#,
    );
    let mut state = state();
    process(&mut state, &function_item);
    discard(&mut state);
    process(&mut state, &done);
    process(&mut state, &done);
    assert_eq!(
        drained(&mut state),
        json(
            r#"[{"type":"content_block_stop","index":0},{"type":"content_block_stop","index":0}]"#
        )
    );
    process(&mut state, &ghost);
    assert_eq!(drained(&mut state), json("[]"));
}

fn done_stops_unmapped_block() {
    let before = mapping(
        r#"{"type":"response.function_call_arguments.delta","item_id":"tool_a","delta":"{}"}"#,
    );
    let unmapped_done = mapping(
        r#"{"type":"response.output_item.done","item":{"type":"function_call","id":"tool_a"}}"#,
    );
    let mut state = state();
    process(&mut state, &before);
    discard(&mut state);
    process(&mut state, &unmapped_done);
    assert_eq!(drained(&mut state), json("[]"));
}

#[test]
fn tool_arguments_and_done() {
    arguments_before_block();
    arguments_deltas();
    done_stops_mapped_block();
    done_stops_unmapped_block();
}

fn refusal_items() -> [Json; 1] {
    [json(
        r#"{"type":"message","content":[{"type":"refusal","refusal":"final-no"}]}"#,
    )]
}

fn call_items() -> [Json; 1] {
    [json(
        r#"{"type":"function_call","call_id":"call_a","name":"lookup","arguments":"{}"}"#,
    )]
}

fn completed_refusal() {
    let refusal_items = refusal_items();
    let output = [OutputItem::Raw(&refusal_items[0])];
    let usage_value = completed_usage();
    let mut state = state();
    completed(
        &mut state,
        "response.completed",
        &output,
        Some("completed"),
        Some(&usage_value),
    );
    assert_eq!(
        drained(&mut state),
        terminal_chunks("refusal", Some("final-no"), projected_usage())
    );
    assert!(state.snapshot().sent_message_stop);
}

fn completed_tool_and_incomplete() {
    let call_items = call_items();
    let call_output = [OutputItem::Raw(&call_items[0])];
    let usage_value = completed_usage();
    let mut state = state();
    completed(
        &mut state,
        "response.completed",
        &call_output,
        Some("completed"),
        Some(&usage_value),
    );
    assert_eq!(
        drained(&mut state),
        terminal_chunks("tool_use", None, projected_usage())
    );
    completed(
        &mut state,
        "response.incomplete",
        &call_output,
        Some("incomplete"),
        Some(&usage_value),
    );
    assert_eq!(
        drained(&mut state),
        terminal_chunks("max_tokens", None, projected_usage())
    );
}

fn completed_text_and_empty() {
    let text_items = [json(
        r#"{"type":"message","content":[{"type":"output_text","text":"hello"}]}"#,
    )];
    let text_output = [OutputItem::Raw(&text_items[0])];
    let usage_value = completed_usage();
    let mut state = state();
    completed(
        &mut state,
        "response.completed",
        &text_output,
        Some("completed"),
        Some(&usage_value),
    );
    assert_eq!(
        drained(&mut state),
        terminal_chunks("end_turn", None, projected_usage())
    );
    completed(
        &mut state,
        "response.completed",
        &[],
        Some("completed"),
        Some(&usage_value),
    );
    assert_eq!(
        drained(&mut state),
        terminal_chunks("end_turn", None, projected_usage())
    );
    completed(
        &mut state,
        "response.completed",
        &[],
        Some("completed"),
        None,
    );
    assert_eq!(
        drained(&mut state),
        terminal_chunks("end_turn", None, zero_usage())
    );
}

fn completed_accumulated_refusal() {
    let refusal_items = refusal_items();
    let output = [OutputItem::Raw(&refusal_items[0])];
    let streamed = mapping(r#"{"type":"response.refusal.delta","item_id":"r","delta":"streamed"}"#);
    let mut state = state();
    process(&mut state, &streamed);
    assert_eq!(
        drained(&mut state),
        json(
            r#"[{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"streamed"}}]"#
        )
    );
    completed(
        &mut state,
        "response.completed",
        &[],
        Some("completed"),
        None,
    );
    assert_eq!(
        drained(&mut state),
        terminal_chunks("refusal", Some("streamed"), zero_usage())
    );
    process(&mut state, &streamed);
    discard(&mut state);
    completed(
        &mut state,
        "response.completed",
        &output,
        Some("completed"),
        None,
    );
    assert_eq!(
        drained(&mut state),
        terminal_chunks("refusal", Some("final-no"), zero_usage())
    );
}

fn completed_missing_response() {
    let event_type = text("response.completed");
    let attributes = EventAttributes {
        event_type: Some(&event_type),
        ..EventAttributes::default()
    };
    let mut state = state();
    process_attribute(&mut state, attributes);
    assert_eq!(
        drained(&mut state),
        terminal_chunks("end_turn", None, zero_usage())
    );
}

fn completed_open_block() {
    let refusal_items = refusal_items();
    let output = [OutputItem::Raw(&refusal_items[0])];
    let usage_value = completed_usage();
    let mut state = state();
    process(&mut state, &mapping(MESSAGE_ITEM));
    assert_eq!(
        drained(&mut state),
        json(
            r#"[{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}]"#
        )
    );
    completed(
        &mut state,
        "response.completed",
        &output,
        Some("completed"),
        Some(&usage_value),
    );
    assert_eq!(
        drained(&mut state),
        terminal_chunks("refusal", Some("final-no"), projected_usage())
    );
    assert!(state.snapshot().chunks.is_empty());
}

#[test]
fn completed_stop_precedence() {
    completed_refusal();
    completed_tool_and_incomplete();
    completed_text_and_empty();
    completed_accumulated_refusal();
    completed_missing_response();
    completed_open_block();
}

fn dictionary_response_reads_attributes() {
    let dictionary = mapping(
        r#"{"type":"response.completed","response":{"status":"incomplete","usage":{"input_tokens":10,"input_tokens_details":{"cached_tokens":3},"output_tokens":2,"total_tokens":12},"output":[{"type":"function_call","call_id":"call_a","name":"lookup","arguments":"{}"}]}}"#,
    );
    let mut state = state();
    process(&mut state, &dictionary);
    assert_eq!(
        drained(&mut state),
        terminal_chunks("end_turn", None, zero_usage())
    );
}

fn attribute_response_uses_its_fields() {
    let usage_value = completed_usage();
    let call_items = call_items();
    let output = [OutputItem::Raw(&call_items[0])];
    let mut state = state();
    completed(
        &mut state,
        "response.completed",
        &output,
        Some("incomplete"),
        Some(&usage_value),
    );
    assert_eq!(
        drained(&mut state),
        terminal_chunks("max_tokens", None, projected_usage())
    );
}

fn dictionary_completed_twice() {
    let dictionary = mapping(r#"{"type":"response.completed"}"#);
    let mut state = state();
    process(&mut state, &dictionary);
    process(&mut state, &dictionary);
    let Json::List(pair) = terminal_chunks("end_turn", None, zero_usage()) else {
        panic!("the terminal pair is a list");
    };
    let mut doubled = pair.clone();
    doubled.extend(pair);
    assert_eq!(drained(&mut state), Json::List(doubled));
}

fn dictionary_completed_without_response() {
    let empty = mapping(r#"{"type":"response.completed","response":null}"#);
    let mut state = state();
    process(&mut state, &empty);
    assert_eq!(
        drained(&mut state),
        terminal_chunks("end_turn", None, zero_usage())
    );
}

#[test]
fn completed_dictionary_shape() {
    dictionary_response_reads_attributes();
    attribute_response_uses_its_fields();
    dictionary_completed_twice();
    dictionary_completed_without_response();
}

const FAILED: &str = r#"{"type":"response.failed","response":{"error":{"type":"rate_limit_error","message":"busy"}}}"#;

fn failed_mapping() {
    let mut state = state();
    process(&mut state, &mapping(FAILED));
    let snapshot = state.snapshot();
    assert_eq!(
        Json::List(snapshot.chunks),
        json(r#"[{"type":"error","error":{"type":"rate_limit_error","message":"busy"}}]"#)
    );
    assert!(snapshot.stream_failed);
    assert!(!snapshot.sent_message_stop);
    assert_eq!(snapshot.logs.len(), 1);
    let log = snapshot.logs.first().expect("the failure log");
    assert_eq!(log.level, "error");
    assert_eq!(
        log.message,
        "AnthropicResponsesStreamWrapper: upstream Responses stream for model_a failed (429): busy"
    );
}

fn failed_defaults() {
    let defaults = [
        r#"{"type":"response.failed"}"#,
        r#"{"type":"response.failed","response":null}"#,
        r#"{"type":"response.failed","response":{"error":null}}"#,
        r#"{"type":"response.failed","response":{"error":5}}"#,
        r#"{"type":"response.failed","response":{"error":"plain"}}"#,
    ];
    for source in defaults {
        let mut state = state();
        process(&mut state, &mapping(source));
        let snapshot = state.snapshot();
        assert_eq!(
            Json::List(snapshot.chunks),
            json(FAILED_DEFAULT),
            "{source}"
        );
        assert!(snapshot.stream_failed);
        assert_eq!(
            snapshot.logs.first().map(|log| log.message.as_str()),
            Some(FAILED_LOG_DEFAULT)
        );
    }
}

fn failed_scalar_responses() {
    for source in [
        r#"{"type":"response.failed","response":5}"#,
        r#"{"type":"response.failed","response":5.5}"#,
        r#"{"type":"response.failed","response":[1]}"#,
        r#"{"type":"response.failed","response":true}"#,
    ] {
        let mut state = state();
        let event = mapping(source);
        let result = state.process_event(ParsedEvent {
            attributes: EventAttributes::default(),
            mapping: Some(&event),
        });
        assert!(
            matches!(result, Err(MessagesResponseEventError::FailedEvent)),
            "{source}"
        );
        assert!(!state.stream_failed());
        assert!(state.next_chunk().is_none());
        assert!(state.snapshot().logs.is_empty());
    }
}

fn failed_attribute() {
    let message = text("busy");
    let error_type = text("rate_limit_error");
    let event_type = text("response.failed");
    let attributes = EventAttributes {
        event_type: Some(&event_type),
        response: Some(ResponseAttribute::Object(ResponseAttributes {
            output: &[],
            status: None,
            usage: None,
            error: Some(ErrorAttribute::Attributes(StreamErrorAttributes {
                message: Some(&message),
                error_type: Some(&error_type),
                code: None,
            })),
        })),
        ..EventAttributes::default()
    };
    let mut state = state();
    process_attribute(&mut state, attributes);
    let snapshot = state.snapshot();
    assert_eq!(
        Json::List(snapshot.chunks),
        json(r#"[{"type":"error","error":{"type":"rate_limit_error","message":"busy"}}]"#)
    );
    assert!(snapshot.stream_failed);
}

fn failed_attribute_mapping_error() {
    let fields = mapping(r#"{"type":"rate_limit_error","message":"busy"}"#);
    let event_type = text("response.failed");
    let attributes = EventAttributes {
        event_type: Some(&event_type),
        response: Some(ResponseAttribute::Object(ResponseAttributes {
            output: &[],
            status: None,
            usage: None,
            error: Some(ErrorAttribute::Mapping(&fields)),
        })),
        ..EventAttributes::default()
    };
    let mut state = state();
    process_attribute(&mut state, attributes);
    assert_eq!(
        Json::List(state.snapshot().chunks),
        json(r#"[{"type":"error","error":{"type":"rate_limit_error","message":"busy"}}]"#)
    );
}

fn failed_attribute_scalar_error() {
    let event_type = text("response.failed");
    let attributes = EventAttributes {
        event_type: Some(&event_type),
        response: Some(ResponseAttribute::Object(ResponseAttributes {
            output: &[],
            status: None,
            usage: None,
            error: Some(ErrorAttribute::Attributes(StreamErrorAttributes::default())),
        })),
        ..EventAttributes::default()
    };
    let mut state = state();
    process_attribute(&mut state, attributes);
    assert_eq!(Json::List(state.snapshot().chunks), json(FAILED_DEFAULT));
}

fn failed_after_content() {
    let ahead =
        mapping(r#"{"type":"response.output_text.delta","item_id":"item_a","delta":"ahead"}"#);
    let mut state = state();
    process(&mut state, &mapping(MESSAGE_ITEM));
    process(&mut state, &ahead);
    process(&mut state, &mapping(FAILED));
    assert_eq!(
        drained(&mut state),
        json(
            r#"[{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"ahead"}},{"type":"error","error":{"type":"rate_limit_error","message":"busy"}}]"#
        )
    );
    assert!(state.snapshot().stream_failed);
    assert!(!state.snapshot().sent_message_stop);
}

fn failed_twice() {
    let mut state = state();
    process(&mut state, &mapping(FAILED));
    process(&mut state, &mapping(FAILED));
    let snapshot = state.snapshot();
    assert_eq!(snapshot.chunks.len(), 2);
    assert_eq!(snapshot.logs.len(), 2);
    assert!(snapshot.stream_failed);
}

#[test]
fn failed_event_terminal() {
    failed_mapping();
    failed_defaults();
    failed_scalar_responses();
    failed_attribute();
    failed_attribute_mapping_error();
    failed_attribute_scalar_error();
    failed_after_content();
    failed_twice();
}
