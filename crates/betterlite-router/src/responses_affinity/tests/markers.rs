use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_core::api::response_id_traversal::record::Record;

use super::*;
use crate::responses_affinity::markers::{
    anthropic_model_id, input_model_id, item_model_id, wrapped_model_id,
};

fn record(value: Json) -> Record {
    Record::from_json(value)
}

#[test]
fn the_input_scan_reads_the_first_encoded_marker_in_order() {
    let input = Json::List(vec![
        message_item(),
        reasoning_item("A"),
        reasoning_item("B"),
    ]);
    assert_eq!(input_model_id(Some(&record(input))), Some("A".to_owned()));
}

#[test]
fn the_input_scan_skips_items_that_are_not_objects() {
    let number = Json::Number(serde_json::Number::from(1_i32));
    let input = Json::List(vec![text("literal"), number, reasoning_item("A")]);
    assert_eq!(input_model_id(Some(&record(input))), Some("A".to_owned()));
}

#[test]
fn a_string_or_absent_input_has_no_marker() {
    assert_eq!(input_model_id(Some(&record(text("plain")))), None);
    assert_eq!(input_model_id(None), None);
}

#[test]
fn an_encoded_id_wins_over_the_wrapped_content() {
    let item = Json::Object(object(&[
        ("id", text(&item_marker("A"))),
        ("encrypted_content", text(&wrapped_marker("B"))),
    ]));
    assert_eq!(item_model_id(&record(item)), Some("A".to_owned()));
}

#[test]
fn an_empty_decoded_id_stops_the_input_scan() {
    let empty_id_item = Json::Object(object(&[("id", text(&item_marker("")))]));
    let input = Json::List(vec![empty_id_item, reasoning_item("B")]);
    assert_eq!(input_model_id(Some(&record(input))), Some(String::new()));
}

#[test]
fn a_plain_item_id_falls_through_to_the_wrapped_content() {
    let item = Json::Object(object(&[
        ("id", text("msg_123")),
        ("encrypted_content", text(&wrapped_marker("A"))),
    ]));
    assert_eq!(item_model_id(&record(item)), Some("A".to_owned()));
}

#[test]
fn an_item_with_only_wrapped_content_carries_the_marker() {
    let item = Json::Object(object(&[("encrypted_content", text(&wrapped_marker("A")))]));
    assert_eq!(item_model_id(&record(item)), Some("A".to_owned()));
}

#[test]
fn a_non_text_or_empty_item_id_falls_through_to_the_wrapped_content() {
    let numeric_id = Json::Object(object(&[
        ("id", Json::Number(serde_json::Number::from(5_i32))),
        ("encrypted_content", text(&wrapped_marker("A"))),
    ]));
    assert_eq!(item_model_id(&record(numeric_id)), Some("A".to_owned()));
    let empty_id = Json::Object(object(&[
        ("id", text("")),
        ("encrypted_content", text(&wrapped_marker("B"))),
    ]));
    assert_eq!(item_model_id(&record(empty_id)), Some("B".to_owned()));
}

#[test]
fn an_empty_wrapped_id_is_not_a_marker() {
    assert_eq!(wrapped_model_id(&wrapped_marker("")), None);
    let item = Json::Object(object(&[("encrypted_content", text(&wrapped_marker("")))]));
    assert_eq!(item_model_id(&record(item)), None);
}

#[test]
fn the_anthropic_scan_reads_thinking_and_redacted_blocks() {
    let thinking = vec![message(vec![text_part("hello"), thinking_part(Some("A"))])];
    assert_eq!(anthropic_model_id(&thinking), Some("A".to_owned()));
    let redacted = vec![message(vec![redacted_part("B")])];
    assert_eq!(anthropic_model_id(&redacted), Some("B".to_owned()));
}

#[test]
fn the_anthropic_scan_ignores_plain_signatures() {
    let plain = vec![message(vec![thinking_part(None), text_part("hello")])];
    assert_eq!(anthropic_model_id(&plain), None);
    assert_eq!(anthropic_model_id(&[]), None);
}

#[test]
fn the_input_marker_wins_over_anthropic_messages() {
    let deployments = vec![
        spec("A", params(Some("u-a"), Some("k-a"))),
        spec("B", params(Some("u-b"), Some("k-b"))),
    ];
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A")]);
    request.messages = Some(vec![message(vec![thinking_part(Some("B"))])]);
    let returned = request.filter(&check, "gpt-group", &deployments).unwrap();
    assert_eq!(ids(&returned), vec![Some("A".to_owned())]);
}

#[test]
fn an_empty_input_marker_falls_through_to_anthropic_messages() {
    let deployments = vec![
        spec("A", params(Some("u-a"), Some("k-a"))),
        spec("B", params(Some("u-b"), Some("k-b"))),
    ];
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let empty_id_item = Json::Object(object(&[("id", text(&item_marker("")))]));
    let mut request = Request::with_items(vec![empty_id_item, reasoning_item("A")]);
    request.messages = Some(vec![message(vec![thinking_part(Some("B"))])]);
    let returned = request.filter(&check, "gpt-group", &deployments).unwrap();
    assert_eq!(ids(&returned), vec![Some("B".to_owned())]);
}

#[test]
fn explicit_messages_win_over_request_kwargs_messages() {
    let deployments = vec![
        spec("A", params(Some("u-a"), Some("k-a"))),
        spec("B", params(Some("u-b"), Some("k-b"))),
    ];
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::new();
    request.messages = Some(vec![message(vec![thinking_part(Some("A"))])]);
    request.kwargs_messages = Some(vec![message(vec![thinking_part(Some("B"))])]);
    let returned = request.filter(&check, "gpt-group", &deployments).unwrap();
    assert_eq!(ids(&returned), vec![Some("A".to_owned())]);
}

#[test]
fn an_empty_messages_argument_falls_back_to_request_kwargs_messages() {
    let deployments = vec![
        spec("A", params(Some("u-a"), Some("k-a"))),
        spec("B", params(Some("u-b"), Some("k-b"))),
    ];
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::new();
    request.messages = Some(Vec::new());
    request.kwargs_messages = Some(vec![message(vec![thinking_part(Some("B"))])]);
    let returned = request.filter(&check, "gpt-group", &deployments).unwrap();
    assert_eq!(ids(&returned), vec![Some("B".to_owned())]);
}
