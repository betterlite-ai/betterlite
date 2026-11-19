use betterlite_models::decisions::Json;

use crate::spend::estimates::decode::decode_spend_estimate;
use crate::spend::estimates::error::EstimateError;

fn issues(error: EstimateError) -> Vec<crate::spend::estimates::error::Issue> {
    match error {
        EstimateError::Request(issues) => issues,
        other => panic!("expected a request error, got {other}"),
    }
}

#[test]
fn decode_accepts_the_three_optional_fields() {
    let request = decode_spend_estimate(
        br#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}],
            "completion_response":{"id":"r"}}"#,
    )
    .unwrap();
    assert_eq!(request.model.as_deref(), Some("gpt-4o-mini"));
    assert_eq!(request.messages.map(|messages| messages.len()), Some(1));
    assert!(request.completion_response.is_some());
}

#[test]
fn decode_accepts_an_empty_object() {
    let request = decode_spend_estimate(b"{}").unwrap();
    assert!(request.model.is_none());
    assert!(request.messages.is_none());
    assert!(request.completion_response.is_none());
}

#[test]
fn decode_keeps_an_empty_message_list_as_supplied() {
    let request = decode_spend_estimate(br#"{"model":"m","messages":[]}"#).unwrap();
    assert_eq!(request.messages, Some(Vec::new()));
}

#[test]
fn decode_keeps_a_null_field_absent() {
    let request =
        decode_spend_estimate(br#"{"model":null,"messages":null,"completion_response":null}"#)
            .unwrap();
    assert!(request.model.is_none());
    assert!(request.messages.is_none());
    assert!(request.completion_response.is_none());
}

#[test]
fn decode_rejects_an_empty_body() {
    let issue = issues_of(
        decode_spend_estimate(b""),
        "missing",
        vec!["body".to_owned()],
    );
    assert_eq!(issue, "Field required");
}

#[test]
fn decode_rejects_malformed_json() {
    let issue = issues_of(
        decode_spend_estimate(b"{"),
        "json_invalid",
        vec!["body".to_owned()],
    );
    assert_eq!(issue, "JSON decode error");
}

#[test]
fn decode_rejects_a_non_object_body() {
    let issue = issues_of(
        decode_spend_estimate(b"[]"),
        "model_attributes_type",
        vec!["body".to_owned()],
    );
    assert_eq!(
        issue,
        "Input should be a valid dictionary or object to extract fields from"
    );
}

#[test]
fn decode_rejects_a_non_string_model() {
    let issue = issues_of(
        decode_spend_estimate(br#"{"model":5}"#),
        "string_type",
        vec!["body".to_owned(), "model".to_owned()],
    );
    assert_eq!(issue, "Input should be a valid string");
}

#[test]
fn decode_rejects_a_non_list_messages() {
    let issue = issues_of(
        decode_spend_estimate(br#"{"messages":{}}"#),
        "list_type",
        vec!["body".to_owned(), "messages".to_owned()],
    );
    assert_eq!(issue, "Input should be a valid list");
}

#[test]
fn decode_rejects_a_non_object_completion_response() {
    let issue = issues_of(
        decode_spend_estimate(br#"{"completion_response":"done"}"#),
        "dict_type",
        vec!["body".to_owned(), "completion_response".to_owned()],
    );
    assert_eq!(issue, "Input should be a valid dictionary");
}

#[test]
fn decode_reports_every_bad_field_at_once() {
    let issues = issues(decode_spend_estimate(br#"{"model":5,"messages":5}"#).unwrap_err());
    assert_eq!(issues.len(), 2);
    assert_eq!(issues[0].kind, "string_type");
    assert_eq!(issues[1].kind, "list_type");
}

#[test]
fn decode_keeps_message_entries_as_they_came() {
    let request =
        decode_spend_estimate(br#"{"model":"m","messages":[1,"two",{"role":"user"}]}"#).unwrap();
    let messages = request.messages.unwrap();
    assert!(matches!(messages[0], Json::Number(_)));
    assert_eq!(messages[1], Json::Text("two".to_owned()));
    assert!(matches!(messages[2], Json::Object(_)));
}

fn issues_of(
    error: Result<impl std::fmt::Debug, EstimateError>,
    kind: &str,
    loc: Vec<String>,
) -> String {
    let issues = issues(error.unwrap_err());
    let issue = issues.first().expect("one issue");
    assert_eq!(issue.kind, kind);
    assert_eq!(issue.loc, loc);
    issue.msg.clone()
}
