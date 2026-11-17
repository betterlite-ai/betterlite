use betterlite_config::sections::pointfive_params::PointFiveInitParams;
use betterlite_models::decisions::Json;

use crate::fields::payload::StandardLoggingPayload;

use super::support::{
    FakeHttp, client, core_with, default_params, event, event_with, fields, payload,
    payload_with_messages,
};
use crate::finops::pointfive::logger::record::record_for;

const MARKER: &str = "redacted-by-litellm";

fn flagged(turn_off_message_logging: bool) -> PointFiveInitParams {
    let mut params = default_params();
    params.turn_off_message_logging = turn_off_message_logging;
    params
}

fn message_content(record: &StandardLoggingPayload) -> String {
    let Some(Json::List(items)) = record.messages.as_ref() else {
        panic!("the record has message items");
    };
    let Json::Object(first) = &items[0] else {
        panic!("the message is an object");
    };
    match &first["content"] {
        Json::Text(content) => content.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            panic!("the message content is text")
        }
    }
}

fn choice_content(record: &StandardLoggingPayload) -> String {
    let Some(Json::Object(response)) = record.response.as_ref() else {
        panic!("the record has a response object");
    };
    let Json::List(choices) = &response["choices"] else {
        panic!("the response has choices");
    };
    let Json::Object(choice) = &choices[0] else {
        panic!("the choice is an object");
    };
    let Json::Object(message) = &choice["message"] else {
        panic!("the choice has a message");
    };
    match &message["content"] {
        Json::Text(content) => content.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            panic!("the choice content is text")
        }
    }
}

fn headers(pairs: &[(&str, bool)]) -> Json {
    let converted: Vec<(&str, Json)> = pairs
        .iter()
        .map(|(name, value)| (*name, Json::Bool(*value)))
        .collect();
    Json::Object(fields(&converted))
}

#[test]
fn the_local_flag_redacts_messages_and_the_response() {
    let http = FakeHttp::default();
    let core = core_with(flagged(true), client(&http, 1), false);
    let record =
        record_for(&core, &event(payload_with_messages("evt-1"))).expect("the record builds");
    assert_eq!(message_content(&record), MARKER);
    assert_eq!(choice_content(&record), MARKER);
}

#[test]
fn a_record_is_unchanged_when_nothing_redacts() {
    let http = FakeHttp::default();
    let core = core_with(flagged(false), client(&http, 1), false);
    let original = payload_with_messages("evt-1");
    let record = record_for(&core, &event(original.clone())).expect("the record builds");
    assert_eq!(record.messages, original.messages);
    assert_eq!(record.response, original.response);
}

#[test]
fn the_global_flag_redacts() {
    let http = FakeHttp::default();
    let core = core_with(flagged(false), client(&http, 1), true);
    let record =
        record_for(&core, &event(payload_with_messages("evt-1"))).expect("the record builds");
    assert_eq!(message_content(&record), MARKER);
    assert_eq!(choice_content(&record), MARKER);
}

#[test]
fn a_dynamic_true_enables_redaction() {
    let http = FakeHttp::default();
    let core = core_with(flagged(false), client(&http, 1), false);
    let event = event_with(
        payload_with_messages("evt-1"),
        &[],
        &[("turn_off_message_logging", Json::Bool(true))],
    );
    let record = record_for(&core, &event).expect("the record builds");
    assert_eq!(message_content(&record), MARKER);
}

#[test]
fn a_dynamic_false_overrides_the_global_flag() {
    let http = FakeHttp::default();
    let core = core_with(flagged(false), client(&http, 1), true);
    let original = payload_with_messages("evt-1");
    let event = event_with(
        original.clone(),
        &[],
        &[("turn_off_message_logging", Json::Bool(false))],
    );
    let record = record_for(&core, &event).expect("the record builds");
    assert_eq!(record.messages, original.messages);
}

#[test]
fn a_dynamic_string_flag_is_read_as_a_bool() {
    let http = FakeHttp::default();
    let core = core_with(flagged(false), client(&http, 1), true);
    let original = payload_with_messages("evt-1");
    let event = event_with(
        original.clone(),
        &[],
        &[("turn_off_message_logging", Json::Text("false".to_owned()))],
    );
    let record = record_for(&core, &event).expect("the record builds");
    assert_eq!(record.messages, original.messages);
}

#[test]
fn a_disable_header_overrides_the_global_flag() {
    let http = FakeHttp::default();
    let core = core_with(flagged(false), client(&http, 1), true);
    let original = payload_with_messages("evt-1");
    let event = event_with(
        original.clone(),
        &[(
            "headers",
            headers(&[("litellm-disable-message-redaction", true)]),
        )],
        &[],
    );
    let record = record_for(&core, &event).expect("the record builds");
    assert_eq!(record.messages, original.messages);
}

#[test]
fn an_enable_header_redacts_without_the_global_flag() {
    let http = FakeHttp::default();
    let core = core_with(flagged(false), client(&http, 1), false);
    let event = event_with(
        payload_with_messages("evt-1"),
        &[(
            "headers",
            headers(&[("x-litellm-enable-message-redaction", true)]),
        )],
        &[],
    );
    let record = record_for(&core, &event).expect("the record builds");
    assert_eq!(message_content(&record), MARKER);
}

#[test]
fn a_redacted_record_keeps_the_other_fields() {
    let http = FakeHttp::default();
    let core = core_with(flagged(true), client(&http, 1), false);
    let record =
        record_for(&core, &event(payload_with_messages("evt-1"))).expect("the record builds");
    assert_eq!(record.id, "evt-1");
    assert_eq!(record.model, "gpt-3.5-turbo");
    assert_eq!(record.total_tokens, 30);
}

#[test]
fn a_redacted_response_keeps_the_original_id_and_created() {
    let http = FakeHttp::default();
    let core = core_with(flagged(true), client(&http, 1), false);
    let record =
        record_for(&core, &event(payload_with_messages("evt-1"))).expect("the record builds");
    let Some(Json::Object(response)) = record.response.as_ref() else {
        panic!("the record has a response object");
    };
    assert_eq!(
        response.get("id"),
        Some(&Json::Text("chatcmpl-1".to_owned()))
    );
    assert_eq!(
        response.get("created"),
        Some(&Json::Number(serde_json::Number::from(1_700_000_000_i64)))
    );
}

#[test]
fn a_record_without_messages_survives_the_local_redaction() {
    let http = FakeHttp::default();
    let core = core_with(flagged(true), client(&http, 1), false);
    let record = record_for(&core, &event(payload("evt-1"))).expect("the record builds");
    assert_eq!(record.id, "evt-1");
    assert!(record.messages.is_none());
}
