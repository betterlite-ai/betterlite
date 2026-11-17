#![cfg(test)]

use super::shared::{
    Harness, Kind, capture_logs, chat, describe, logged_text, ok, request_data, tokens_request,
};

#[tokio::test]
async fn output_parse_pii_with_tokens_unmasks_the_stream() {
    let mut harness = Harness::new();
    harness.analyzer.settings.output_parse_pii = true;
    let mut request = tokens_request();
    let (sent, result) = harness
        .run(Kind::Hook, &mut request, ok(vec![chat("Hi <PERSON_1>")]))
        .await;
    result.unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains(r#""content":"Hi Alice""#));
}

#[tokio::test]
async fn output_parse_pii_without_tokens_relays_the_stream_and_logs_at_debug() {
    let (captured, _guard) = capture_logs();
    let mut harness = Harness::new();
    harness.analyzer.settings.output_parse_pii = true;
    let mut request = request_data(r#"{"messages":[]}"#);
    let events = ok(vec![chat("a"), chat("b")]);
    let (sent, result) = harness.run(Kind::Hook, &mut request, events).await;
    result.unwrap();
    assert_eq!(sent, [describe(&chat("a")), describe(&chat("b"))]);
    assert!(logged_text(&captured).contains("No pii_tokens in request_data['metadata']"));
}

#[tokio::test]
async fn an_empty_request_logs_no_missing_token_note() {
    let (captured, _guard) = capture_logs();
    let harness = Harness::new();
    let mut request = request_data("{}");
    let (_sent, result) = harness
        .run(Kind::Hook, &mut request, ok(vec![chat("a")]))
        .await;
    result.unwrap();
    assert!(!logged_text(&captured).contains("No pii_tokens"));
}

#[tokio::test]
async fn tokens_without_output_parse_pii_leave_the_stream_alone() {
    let harness = Harness::new();
    let mut request = tokens_request();
    let (sent, result) = harness
        .run(Kind::Hook, &mut request, ok(vec![chat("Hi <PERSON_1>")]))
        .await;
    result.unwrap();
    assert_eq!(sent, [describe(&chat("Hi <PERSON_1>"))]);
}

#[tokio::test]
async fn the_global_output_parse_flag_does_not_start_unmasking_a_stream() {
    let mut harness = Harness::new();
    harness.global_output_parse_pii = true;
    let mut request = tokens_request();
    let (sent, result) = harness
        .run(Kind::Hook, &mut request, ok(vec![chat("Hi <PERSON_1>")]))
        .await;
    result.unwrap();
    assert_eq!(sent, [describe(&chat("Hi <PERSON_1>"))]);
}
