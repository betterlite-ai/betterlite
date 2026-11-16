#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;

use super::support::{field, frame, json, number, object, project, state_with_pending, uuids};
use crate::gemini_live_accounting::error::LiveAccountingError;
use crate::gemini_live_accounting::response_done::{
    ResponseDoneEvent, ResponseDoneInput, transform_response_done_event,
};
use crate::gemini_live_client::state::LiveSessionState;

const SETUP: &str = r#"{"setup": {"generationConfig": {"temperature": 0.7, "maxOutputTokens": 100, "responseModalities": ["TEXT"]}}}"#;
const REPORTED: &str = r#"{"promptTokenCount": 10, "candidatesTokenCount": 5, "totalTokenCount": 15,
    "promptTokensDetails": [{"modality": "AUDIO", "tokenCount": 10}]}"#;

fn input<'input>(setup: Option<&'input str>, items: &'input [Object]) -> ResponseDoneInput<'input> {
    ResponseDoneInput {
        response_id: Some("resp_given"),
        conversation_id: Some("conv_given"),
        output_items: items,
        session_configuration_request: setup,
    }
}

fn done(
    state: &mut LiveSessionState,
    source: &str,
    input: &ResponseDoneInput<'_>,
) -> Result<ResponseDoneEvent, LiveAccountingError> {
    transform_response_done_event(state, &mut frame(source), input, &project, &mut uuids())
}

fn rendered(event: &ResponseDoneEvent) -> Object {
    let text = serde_json::to_string(event).expect("the event serializes");
    serde_json::from_str(&text).expect("the event is an object")
}

fn response(event: &ResponseDoneEvent) -> Object {
    let Json::Object(response) = field(&rendered(event), "response").clone() else {
        panic!("the response is an object");
    };
    response
}

fn usage(event: &ResponseDoneEvent) -> Object {
    let Json::Object(usage) = field(&response(event), "usage").clone() else {
        panic!("the usage is an object");
    };
    usage
}

#[test]
fn a_frame_with_usage_and_a_setup_completes_the_whole_event() {
    let items = [object(r#"{"id": "item_1", "type": "message"}"#)];
    let event = done(
        &mut LiveSessionState::default(),
        &format!(r#"{{"usageMetadata": {REPORTED}}}"#),
        &input(Some(SETUP), &items),
    )
    .expect("the event builds");
    let expected = json(
        r#"{"type": "response.done", "event_id": "event_01010101-0101-0101-0101-010101010101",
            "response": {"object": "realtime.response", "id": "resp_given", "status": "completed",
                "status_details": null, "output": [{"id": "item_1", "type": "message"}],
                "conversation_id": "conv_given", "modalities": ["text"],
                "usage": {"input_tokens": 10, "output_tokens": 5, "total_tokens": 15,
                    "input_tokens_details": {"audio_tokens": 10, "cached_tokens": 0, "cached_tokens_details": null,
                        "image_tokens": null, "text_tokens": null, "video_tokens": null},
                    "output_tokens_details": {"audio_tokens": null, "reasoning_tokens": null, "text_tokens": 5},
                    "cost": null,
                    "input_token_details": {"cached_tokens": 0, "text_tokens": 0, "audio_tokens": 10},
                    "output_token_details": {"text_tokens": 5, "audio_tokens": 0}},
                "temperature": 0.7, "max_output_tokens": 100}}"#,
    );
    assert_eq!(Json::Object(rendered(&event)), expected);
}

#[test]
fn missing_conversation_and_response_ids_are_generated_in_that_order_before_the_event_id() {
    let event = done(
        &mut LiveSessionState::default(),
        "{}",
        &ResponseDoneInput {
            response_id: None,
            conversation_id: None,
            output_items: &[],
            session_configuration_request: None,
        },
    )
    .expect("the event builds");
    assert_eq!(
        event.response.conversation_id,
        "conv_01010101-0101-0101-0101-010101010101"
    );
    assert_eq!(
        event.response.id,
        "resp_02020202-0202-0202-0202-020202020202"
    );
    assert_eq!(event.event_id, "event_03030303-0303-0303-0303-030303030303");
}

#[rstest]
#[case::no_setup(None, &["audio"])]
#[case::empty_request(Some(""), &["audio"])]
#[case::setup_without_modalities(Some(r#"{"setup": {"generationConfig": {}}}"#), &["audio"])]
#[case::no_generation_config(Some(r#"{"setup": {}}"#), &["audio"])]
#[case::lowered(
    Some(r#"{"setup": {"generationConfig": {"responseModalities": ["TEXT", "Audio"]}}}"#),
    &["text", "audio"]
)]
fn modalities_default_to_audio_and_are_lowercased(
    #[case] setup: Option<&str>,
    #[case] expected: &[&str],
) {
    let event =
        done(&mut LiveSessionState::default(), "{}", &input(setup, &[])).expect("the event builds");
    assert_eq!(event.response.modalities, expected);
}

#[rstest]
#[case::absent(r#"{"setup": {"generationConfig": {}}}"#, false, false)]
#[case::temperature_only(r#"{"setup": {"generationConfig": {"temperature": 0}}}"#, true, false)]
#[case::max_tokens_only(
    r#"{"setup": {"generationConfig": {"maxOutputTokens": 0}}}"#,
    false,
    true
)]
#[case::explicit_nulls(
    r#"{"setup": {"generationConfig": {"temperature": null, "maxOutputTokens": null}}}"#,
    false,
    false
)]
fn temperature_and_max_output_tokens_appear_only_when_set(
    #[case] setup: &str,
    #[case] has_temperature: bool,
    #[case] has_max_tokens: bool,
) {
    let event = done(
        &mut LiveSessionState::default(),
        "{}",
        &input(Some(setup), &[]),
    )
    .expect("the event builds");
    let response = response(&event);
    assert_eq!(response.contains_key("temperature"), has_temperature);
    assert_eq!(response.contains_key("max_output_tokens"), has_max_tokens);
}

#[test]
fn a_zero_temperature_and_zero_max_tokens_are_kept_as_given() {
    let setup = r#"{"setup": {"generationConfig": {"temperature": 0, "maxOutputTokens": 0}}}"#;
    let event = done(
        &mut LiveSessionState::default(),
        "{}",
        &input(Some(setup), &[]),
    )
    .expect("the event builds");
    let response = response(&event);
    assert_eq!(field(&response, "temperature"), &number(0));
    assert_eq!(field(&response, "max_output_tokens"), &number(0));
}

#[test]
fn a_response_without_usage_reports_zero_tokens_and_zero_detail_aliases() {
    let event =
        done(&mut LiveSessionState::default(), "{}", &input(None, &[])).expect("the event builds");
    let usage = usage(&event);
    assert_eq!(field(&usage, "input_tokens"), &number(0));
    assert_eq!(field(&usage, "total_tokens"), &number(0));
    assert_eq!(field(&usage, "input_tokens_details"), &Json::Null);
    assert_eq!(
        field(&usage, "input_token_details"),
        &json(r#"{"cached_tokens": 0, "text_tokens": 0, "audio_tokens": 0}"#)
    );
    assert_eq!(
        field(&usage, "output_token_details"),
        &json(r#"{"text_tokens": 0, "audio_tokens": 0}"#)
    );
}

#[test]
fn search_grounding_from_the_frame_bills_into_the_input_details() {
    let source = format!(
        r#"{{"usageMetadata": {REPORTED}, "serverContent": {{"groundingMetadata": {{"webSearchQueries": ["a", "b"]}}}}}}"#
    );
    let event = done(&mut LiveSessionState::default(), &source, &input(None, &[]))
        .expect("the event builds");
    let usage = usage(&event);
    let Json::Object(details) = field(&usage, "input_tokens_details") else {
        panic!("the input details are an object");
    };
    assert_eq!(field(details, "web_search_requests"), &number(2));
    let Json::Object(alias) = field(&usage, "input_token_details") else {
        panic!("the alias is an object");
    };
    assert_eq!(field(alias, "web_search_requests"), &number(2));
}

#[test]
fn grounding_in_a_frame_without_usage_bills_nothing() {
    let source = r#"{"serverContent": {"groundingMetadata": {"webSearchQueries": ["a"]}}}"#;
    let event = done(&mut LiveSessionState::default(), source, &input(None, &[]))
        .expect("the event builds");
    assert_eq!(field(&usage(&event), "input_tokens_details"), &Json::Null);
}

#[test]
fn pending_usage_is_billed_by_the_first_response_only() {
    let mut state = state_with_pending(REPORTED);
    let first = done(&mut state, "{}", &input(None, &[])).expect("the first event builds");
    let second = done(&mut state, "{}", &input(None, &[])).expect("the second event builds");
    assert_eq!(field(&usage(&first), "total_tokens"), &number(15));
    assert_eq!(field(&usage(&second), "total_tokens"), &number(0));
}

#[test]
fn an_empty_usage_dictionary_in_the_frame_beats_the_pending_buffer() {
    let mut state = state_with_pending(REPORTED);
    let event =
        done(&mut state, r#"{"usageMetadata": {}}"#, &input(None, &[])).expect("the event builds");
    let usage = usage(&event);
    assert_eq!(field(&usage, "total_tokens"), &number(0));
    assert_ne!(field(&usage, "input_tokens_details"), &Json::Null);
    assert_eq!(state.pending_usage_metadata, None);
}

#[test]
fn a_setup_that_is_not_json_fails_before_any_usage_is_consumed() {
    let mut state = state_with_pending(REPORTED);
    let failure = done(&mut state, "{}", &input(Some("not json"), &[]));
    assert!(matches!(failure, Err(LiveAccountingError::Setup(_))));
    assert!(state.pending_usage_metadata.is_some());
}

#[test]
fn usage_metadata_the_chat_calculation_refuses_is_an_error() {
    let source = r#"{"usageMetadata": {"promptTokenCount": 9223372036854775807, "toolUsePromptTokenCount": 1}}"#;
    let failure = done(&mut LiveSessionState::default(), source, &input(None, &[]));
    assert!(matches!(failure, Err(LiveAccountingError::Usage(_))));
}
