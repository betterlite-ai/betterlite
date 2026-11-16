use std::sync::Mutex;

use crate::passthrough_reconstruction::error::ReconstructionError;
use crate::passthrough_reconstruction::events::RawSseChunk;
use crate::passthrough_reconstruction::usage::{UsageOnlyRequest, build_usage_only_response};
use crate::streamed_response_assembly::response::{ChatResponse, Response};

use super::support::{FIXED_MODEL, FIXED_NOW, FixtureCalculator, fixed_id, text_chunk};

const NO_USAGE: &str = "{}";

fn request<'request>(
    chunks: &'request [RawSseChunk],
    calculator: &'request FixtureCalculator,
    model: &'request str,
    speed: Option<&'request str>,
) -> UsageOnlyRequest<'request, FixtureCalculator> {
    UsageOnlyRequest {
        chunks,
        model,
        speed,
        calculator,
        now: FIXED_NOW,
        next_id: fixed_id,
    }
}

fn start_event(usage: &str) -> String {
    format!(
        "data: {{\"type\":\"message_start\",\"message\":{{\"model\":\"{FIXED_MODEL}\",\"usage\":{usage}}}}}"
    )
}

fn start_event_without_model(usage: &str) -> String {
    format!("data: {{\"type\":\"message_start\",\"message\":{{\"usage\":{usage}}}}}")
}

fn delta_event(stop_reason: &str, usage: &str) -> String {
    format!(
        "data: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"{stop_reason}\"}},\"usage\":{usage}}}"
    )
}

fn built(chunks: &[RawSseChunk], calculator: &FixtureCalculator) -> Option<Response> {
    build_usage_only_response(&request(chunks, calculator, "unknown", None)).unwrap()
}

fn chat_of(response: Option<Response>) -> ChatResponse {
    let Some(Response::Chat(chat)) = response else {
        panic!("the usage path builds a chat response");
    };
    *chat
}

#[test]
fn answers_none_without_a_usage_event() {
    let calculator = FixtureCalculator::default();
    let chunks = [
        text_chunk("data: [DONE]"),
        text_chunk("data: {not json}"),
        text_chunk("data: {\"type\":\"content_block_delta\",\"index\":0}"),
        text_chunk("event: message_start"),
        text_chunk("data: [1,2]"),
        text_chunk("data: 5"),
    ];
    assert!(built(&chunks, &calculator).is_none());
}

fn full_stream() -> [RawSseChunk; 2] {
    [
        text_chunk(&start_event(
            "{\"input_tokens\":10,\"cache_read_input_tokens\":3,\"cache_creation_input_tokens\":0,\"cache_creation\":{\"ephemeral_5m_input_tokens\":2,\"ephemeral_1h_input_tokens\":4},\"inference_geo\":\"us\",\"speed\":\"fast\",\"output_tokens\":1}",
        )),
        text_chunk(&delta_event(
            "end_turn",
            "{\"output_tokens\":9,\"server_tool_use\":{\"web_search_requests\":2}}",
        )),
    ]
}

#[test]
fn builds_the_usage_object_from_the_stream() {
    let calculator = FixtureCalculator::default();
    assert!(built(&full_stream(), &calculator).is_some());
    assert_eq!(
        calculator.seen_object(),
        "{\"input_tokens\":10,\"output_tokens\":9,\"cache_read_input_tokens\":3,\"cache_creation_input_tokens\":6,\"cache_creation\":{\"ephemeral_5m_input_tokens\":2,\"ephemeral_1h_input_tokens\":4},\"server_tool_use\":{\"web_search_requests\":2},\"inference_geo\":\"us\",\"speed\":\"fast\"}"
    );
    assert_eq!(calculator.seen_speed(), None);
}

#[test]
fn builds_the_chat_response_around_the_calculated_usage() {
    let calculator = FixtureCalculator::default();
    let chat = chat_of(built(&full_stream(), &calculator));
    assert_eq!(chat.id, "chatcmpl-00000000-0000-0000-0000-000000000001");
    assert_eq!(chat.object, "chat.completion");
    assert_eq!(chat.created, FIXED_NOW);
    assert_eq!(chat.model, FIXED_MODEL);
    assert_eq!(chat.choices[0].index, 0);
    assert_eq!(chat.choices[0].finish_reason, "stop");
    assert_eq!(chat.choices[0].message.role, "assistant");
    assert_eq!(chat.choices[0].message.content.as_deref(), Some(""));
    assert_eq!(chat.usage.prompt_tokens, 11);
    assert_eq!(chat.usage.completion_tokens, 4);
    assert_eq!(chat.usage.total_tokens, 15);
    assert!(chat.system_fingerprint.is_none());
    assert!(chat.service_tier.is_none());
    assert!(chat.hidden_params.is_empty());
}

#[test]
fn keeps_the_given_model_and_passes_the_speed_fallback() {
    let calculator = FixtureCalculator::default();
    let chunks = [text_chunk(&start_event(NO_USAGE))];
    let response = build_usage_only_response(&request(
        &chunks,
        &calculator,
        "claude-3-opus",
        Some("standard"),
    ))
    .unwrap();
    let chat = chat_of(response);
    assert_eq!(chat.model, "claude-3-opus");
    assert_eq!(calculator.seen_speed(), Some("standard".to_owned()));
    assert_eq!(
        calculator.seen_object(),
        "{\"input_tokens\":0,\"output_tokens\":0}"
    );
}

#[rstest::rstest]
#[case::empty("")]
#[case::unknown("unknown")]
fn resolves_an_empty_or_unknown_model_from_the_message(#[case] model: &str) {
    let calculator = FixtureCalculator::default();
    let chunks = [text_chunk(&start_event(NO_USAGE))];
    let response = build_usage_only_response(&request(&chunks, &calculator, model, None)).unwrap();
    assert_eq!(chat_of(response).model, FIXED_MODEL);
}

#[test]
fn keeps_the_model_when_the_message_has_none() {
    let calculator = FixtureCalculator::default();
    let nameless = [text_chunk(&start_event_without_model(NO_USAGE))];
    let from_nameless =
        build_usage_only_response(&request(&nameless, &calculator, "unknown", None)).unwrap();
    assert_eq!(chat_of(from_nameless).model, "unknown");
    let numeric = [text_chunk(
        "data: {\"type\":\"message_start\",\"message\":{\"model\":5}}",
    )];
    let from_numeric =
        build_usage_only_response(&request(&numeric, &calculator, "unknown", None)).unwrap();
    assert_eq!(chat_of(from_numeric).model, "unknown");
    let empty = [text_chunk(
        "data: {\"type\":\"message_start\",\"message\":{\"model\":\"\"}}",
    )];
    let from_empty =
        build_usage_only_response(&request(&empty, &calculator, "unknown", None)).unwrap();
    assert_eq!(chat_of(from_empty).model, "unknown");
}

#[rstest::rstest]
#[case::total_wins(
    "{\"cache_creation_input_tokens\":5,\"cache_creation\":{\"ephemeral_5m_input_tokens\":2,\"ephemeral_1h_input_tokens\":4}}",
    "\"cache_creation_input_tokens\":5"
)]
#[case::split_derives(
    "{\"cache_creation\":{\"ephemeral_5m_input_tokens\":2,\"ephemeral_1h_input_tokens\":4}}",
    "\"cache_creation_input_tokens\":6"
)]
#[case::neither("{}", "\"input_tokens\":0,\"output_tokens\":0}")]
fn derives_the_cache_creation_total_only_from_the_split(
    #[case] usage: &str,
    #[case] expected: &str,
) {
    let calculator = FixtureCalculator::default();
    let chunks = [text_chunk(&start_event(usage))];
    assert!(built(&chunks, &calculator).is_some(), "{usage}");
    assert!(calculator.seen_object().contains(expected), "{usage}");
    assert_eq!(
        calculator.seen_object().contains("\"cache_creation\":"),
        usage.contains("\"cache_creation\":"),
        "{usage}"
    );
}

#[test]
fn keeps_a_one_sided_cache_split() {
    let calculator = FixtureCalculator::default();
    let chunks = [text_chunk(&start_event(
        "{\"cache_creation\":{\"ephemeral_5m_input_tokens\":2}}",
    ))];
    assert!(built(&chunks, &calculator).is_some());
    assert_eq!(
        calculator.seen_object(),
        "{\"input_tokens\":0,\"output_tokens\":0,\"cache_creation_input_tokens\":2,\"cache_creation\":{\"ephemeral_5m_input_tokens\":2,\"ephemeral_1h_input_tokens\":0}}"
    );
}

#[test]
fn carries_server_tool_use_only_when_a_count_is_present() {
    let calculator = FixtureCalculator::default();
    let partial = [
        text_chunk(&start_event(NO_USAGE)),
        text_chunk(&delta_event(
            "tool_use",
            "{\"server_tool_use\":{\"tool_search_requests\":1}}",
        )),
    ];
    let partial_chat = chat_of(built(&partial, &calculator));
    assert_eq!(
        calculator.seen_object(),
        "{\"input_tokens\":0,\"output_tokens\":0,\"server_tool_use\":{\"tool_search_requests\":1}}"
    );
    assert_eq!(partial_chat.choices[0].finish_reason, "tool_calls");
    let zero = [
        text_chunk(&start_event(NO_USAGE)),
        text_chunk(&delta_event(
            "end_turn",
            "{\"server_tool_use\":{\"web_search_requests\":0,\"tool_search_requests\":null}}",
        )),
    ];
    let zero_chat = chat_of(built(&zero, &calculator));
    assert_eq!(
        calculator.seen_object(),
        "{\"input_tokens\":0,\"output_tokens\":0,\"server_tool_use\":{\"web_search_requests\":0}}"
    );
    assert_eq!(zero_chat.choices[0].finish_reason, "stop");
}

#[test]
fn keeps_the_last_output_count_and_ignores_null_and_text() {
    let calculator = FixtureCalculator::default();
    let chunks = [
        text_chunk(&start_event("{\"output_tokens\":1}")),
        text_chunk(&delta_event("end_turn", "{\"output_tokens\":null}")),
        text_chunk(&delta_event("end_turn", "{\"output_tokens\":\"9\"}")),
    ];
    assert!(built(&chunks, &calculator).is_some());
    assert_eq!(
        calculator.seen_object(),
        "{\"input_tokens\":0,\"output_tokens\":1}"
    );
    let later = [
        text_chunk(&start_event("{\"output_tokens\":1}")),
        text_chunk(&delta_event("end_turn", "{\"output_tokens\":9}")),
    ];
    assert!(built(&later, &calculator).is_some());
    assert_eq!(
        calculator.seen_object(),
        "{\"input_tokens\":0,\"output_tokens\":9}"
    );
}

#[test]
fn keeps_the_cache_read_geo_and_speed_a_message_delta_carries() {
    let calculator = FixtureCalculator::default();
    let chunks = [
        text_chunk(&start_event("{\"cache_read_input_tokens\":3}")),
        text_chunk(&delta_event(
            "end_turn",
            "{\"cache_read_input_tokens\":7,\"inference_geo\":\"eu\",\"speed\":\"slow\"}",
        )),
    ];
    assert!(built(&chunks, &calculator).is_some());
    assert_eq!(
        calculator.seen_object(),
        "{\"input_tokens\":0,\"output_tokens\":0,\"cache_read_input_tokens\":7,\"inference_geo\":\"eu\",\"speed\":\"slow\"}"
    );
}

#[test]
fn builds_from_either_usage_event_alone() {
    let calculator = FixtureCalculator::default();
    let start_only = [text_chunk(&start_event(NO_USAGE))];
    assert!(built(&start_only, &calculator).is_some());
    let delta_only = [text_chunk(&delta_event("end_turn", NO_USAGE))];
    let chat = chat_of(built(&delta_only, &calculator));
    assert_eq!(chat.model, "unknown");
    assert_eq!(chat.choices[0].finish_reason, "stop");
}

#[test]
fn uses_stop_for_an_empty_stop_reason() {
    let calculator = FixtureCalculator::default();
    let chunks = [text_chunk(&delta_event("", NO_USAGE))];
    let chat = chat_of(built(&chunks, &calculator));
    assert_eq!(chat.choices[0].finish_reason, "stop");
}

#[test]
fn keeps_the_earlier_stop_reason_across_later_deltas() {
    let calculator = FixtureCalculator::default();
    let chunks = [
        text_chunk(&delta_event("tool_use", NO_USAGE)),
        text_chunk(&delta_event("", NO_USAGE)),
        text_chunk("data: {\"type\":\"message_delta\",\"delta\":{},\"usage\":{}}"),
    ];
    let chat = chat_of(built(&chunks, &calculator));
    assert_eq!(chat.choices[0].finish_reason, "tool_calls");
}

#[test]
fn propagates_the_calculator_failure() {
    let calculator = FixtureCalculator {
        seen_object: Mutex::new(None),
        seen_speed: Mutex::new(None),
        fail: true,
    };
    let chunks = [text_chunk(&start_event(NO_USAGE))];
    let result = build_usage_only_response(&request(&chunks, &calculator, "unknown", None));
    assert!(matches!(result, Err(ReconstructionError::Usage { .. })));
}

#[test]
fn refuses_a_chunk_that_is_not_utf8() {
    let calculator = FixtureCalculator::default();
    let chunks = [RawSseChunk::Bytes(vec![0xff])];
    let result = build_usage_only_response(&request(&chunks, &calculator, "unknown", None));
    assert!(matches!(result, Err(ReconstructionError::Event(_))));
}

#[test]
fn ignores_a_null_usage_object() {
    let calculator = FixtureCalculator::default();
    let chunks = [
        text_chunk(
            "data: {\"type\":\"message_start\",\"message\":{\"model\":\"x\",\"usage\":null}}",
        ),
        text_chunk(&delta_event("end_turn", "null")),
    ];
    let chat = chat_of(built(&chunks, &calculator));
    assert_eq!(chat.model, "x");
    assert_eq!(
        calculator.seen_object(),
        "{\"input_tokens\":0,\"output_tokens\":0}"
    );
}
