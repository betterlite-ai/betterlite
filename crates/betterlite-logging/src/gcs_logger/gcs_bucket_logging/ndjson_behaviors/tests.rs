#![cfg(test)]

use betterlite_models::decisions::Json;

use super::super::tests::support::item;
use super::combine_payloads_to_ndjson;

#[test]
fn each_payload_becomes_one_line() {
    let items = vec![
        item("chatcmpl-1", None, Json::Null),
        item("chatcmpl-2", None, Json::Null),
    ];
    let ndjson = combine_payloads_to_ndjson(&items).expect("the payloads serialize");
    assert_eq!(ndjson.split('\n').count(), 2);
    assert!(!ndjson.ends_with('\n'));
}

#[test]
fn a_line_is_the_payload_as_json() {
    let items = vec![item("chatcmpl-1", None, Json::Null)];
    let ndjson = combine_payloads_to_ndjson(&items).expect("the payloads serialize");
    let parsed: Json = serde_json::from_str(&ndjson).expect("the line is JSON");
    let Json::Object(fields) = parsed else {
        panic!("the line is a JSON object");
    };
    assert_eq!(fields["id"], Json::Text("chatcmpl-1".to_owned()));
    assert_eq!(fields["status"], Json::Text("success".to_owned()));
    assert_eq!(fields["error_str"], Json::Null);
    assert_eq!(fields["model"], Json::Text("gpt-4o".to_owned()));
}

#[test]
fn a_failure_line_keeps_the_error_string() {
    let items = vec![item(
        "chatcmpl-1",
        Some("the upstream call failed"),
        Json::Null,
    )];
    let ndjson = combine_payloads_to_ndjson(&items).expect("the payloads serialize");
    let parsed: Json = serde_json::from_str(&ndjson).expect("the line is JSON");
    let Json::Object(fields) = parsed else {
        panic!("the line is a JSON object");
    };
    assert_eq!(fields["status"], Json::Text("failure".to_owned()));
    assert_eq!(
        fields["error_str"],
        Json::Text("the upstream call failed".to_owned())
    );
}

#[test]
fn no_items_make_an_empty_document() {
    let ndjson = combine_payloads_to_ndjson(&[]).expect("no payload needs no serialization");
    assert_eq!(ndjson, "");
}

#[test]
fn non_ascii_text_is_not_escaped() {
    let items = vec![item("chatcmpl-é", None, Json::Null)];
    let ndjson = combine_payloads_to_ndjson(&items).expect("the payloads serialize");
    assert!(ndjson.contains("chatcmpl-é"));
}
