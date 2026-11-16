#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use rstest::rstest;

use super::{extract_response, response_has_text};
use crate::guardrails::chat_guardrail_translation::rewrites::process_output_response;
use crate::guardrails::chat_guardrail_translation::rewrites::response::ChatResponse;
use crate::guardrails::chat_guardrail_translation::rewrites::tests::support::{
    FakeGuardrail, object, principal, response,
};

const TWO_CHOICES: &str = r#"{"model": "gpt-4o", "choices": [
    {"index": 0, "message": {"content": "hello", "tool_calls": [
        {"id": "call_1", "type": "function", "function": {"name": "lookup", "arguments": "{\"q\":1}"}}]}},
    {"index": 1, "message": {"content": [
        {"type": "text", "text": "part one"},
        {"type": "image_url", "image_url": {"url": "https://img/1.png"}},
        {"type": "text", "text": ""}]}}]}"#;

fn upper_cased(inputs: GuardrailApiInputs) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: inputs
            .texts
            .map(|texts| texts.iter().map(|text| text.to_uppercase()).collect()),
        ..inputs
    }
}

#[test]
fn process_output_extracts_text_images_and_tool_calls_per_choice() {
    let extracted = extract_response(&response(TWO_CHOICES));
    assert_eq!(extracted.texts, ["hello", "part one"]);
    assert_eq!(extracted.text_mappings, [(0, None), (1, Some(0))]);
    assert_eq!(extracted.images, ["https://img/1.png"]);
    assert_eq!(extracted.tool_call_mappings, [(0, 0)]);
    assert_eq!(
        extracted.tool_calls,
        [object(
            r#"{"id": "call_1", "type": "function", "function": {"name": "lookup", "arguments": "{\"q\":1}"}}"#
        )]
    );
}

#[test]
fn process_output_converts_a_tool_call_without_a_type_or_id() {
    let extracted = extract_response(&response(
        r#"{"choices": [{"message": {"tool_calls": [{"function": {"name": "f"}}, {"id": "x"}]}}]}"#,
    ));
    assert_eq!(
        extracted.tool_calls,
        [object(
            r#"{"id": null, "type": "function", "function": {"name": "f", "arguments": null}}"#
        )]
    );
    assert_eq!(extracted.tool_call_mappings, [(0, 0)]);
}

#[rstest]
#[case::string_content(r#"{"choices": [{"message": {"content": "hi"}}]}"#, true)]
#[case::empty_string(r#"{"choices": [{"message": {"content": ""}}]}"#, false)]
#[case::list_content(r#"{"choices": [{"message": {"content": [{"text": "hi"}]}}]}"#, false)]
#[case::tool_calls(
    r#"{"choices": [{"message": {"tool_calls": [{"function": {"name": "f"}}]}}]}"#,
    true
)]
#[case::no_tool_calls(r#"{"choices": [{"message": {"tool_calls": []}}]}"#, false)]
#[case::no_choices(r#"{"choices": []}"#, false)]
fn process_output_detects_content_worth_scanning(#[case] source: &str, #[case] expected: bool) {
    assert_eq!(response_has_text(&response(source)), expected);
}

#[tokio::test]
async fn process_output_response_skips_a_response_with_nothing_to_scan() {
    let guardrail = FakeGuardrail::passing_through();
    let mut subject = response(r#"{"choices": [{"message": {"content": [{"text": "hi"}]}}]}"#);
    let before = subject.clone();
    process_output_response(&mut subject, &guardrail, &principal())
        .await
        .unwrap();
    assert_eq!(subject, before);
    assert!(guardrail.calls().is_empty());
}

#[tokio::test]
async fn process_output_response_skips_a_response_whose_only_content_is_empty_text() {
    let guardrail = FakeGuardrail::passing_through();
    let mut subject =
        response(r#"{"choices": [{"message": {"content": "", "tool_calls": [{"id": "x"}]}}]}"#);
    process_output_response(&mut subject, &guardrail, &principal())
        .await
        .unwrap();
    assert!(guardrail.calls().is_empty());
}

#[tokio::test]
async fn process_output_response_rewrites_each_choice_in_place() {
    let guardrail = FakeGuardrail::answering(Box::new(upper_cased));
    let mut subject = response(TWO_CHOICES);
    process_output_response(&mut subject, &guardrail, &principal())
        .await
        .unwrap();
    let expected: ChatResponse = response(
        &TWO_CHOICES
            .replace("hello", "HELLO")
            .replace("part one", "PART ONE"),
    );
    assert_eq!(subject, expected);
}

#[tokio::test]
async fn process_output_response_sends_the_scanned_content_as_a_response_input() {
    let guardrail = FakeGuardrail::passing_through();
    let mut subject = response(TWO_CHOICES);
    process_output_response(&mut subject, &guardrail, &principal())
        .await
        .unwrap();
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.input_type, GuardrailInputType::Response);
    assert_eq!(
        call.inputs.texts,
        Some(vec!["hello".to_owned(), "part one".to_owned()])
    );
    assert_eq!(
        call.inputs.images,
        Some(vec!["https://img/1.png".to_owned()])
    );
    assert_eq!(call.inputs.model.as_deref(), Some("gpt-4o"));
    assert_eq!(call.inputs.tool_calls.as_ref().map(Vec::len), Some(1));
    assert_eq!(call.inputs.tools, None);
    assert_eq!(call.inputs.structured_messages, None);
}

#[tokio::test]
async fn process_output_response_leaves_out_empty_images_tool_calls_and_model() {
    let guardrail = FakeGuardrail::passing_through();
    let mut subject = response(r#"{"model": "", "choices": [{"message": {"content": "hi"}}]}"#);
    process_output_response(&mut subject, &guardrail, &principal())
        .await
        .unwrap();
    let inputs = &guardrail.calls()[0].inputs;
    assert_eq!(inputs.images, None);
    assert_eq!(inputs.tool_calls, None);
    assert_eq!(inputs.model, None);
}

#[tokio::test]
async fn process_output_response_sends_tool_calls_with_an_empty_text_list() {
    let guardrail = FakeGuardrail::passing_through();
    let mut subject = response(
        r#"{"choices": [{"message": {"tool_calls": [{"id": "a", "function": {"name": "f", "arguments": "{}"}}]}}]}"#,
    );
    process_output_response(&mut subject, &guardrail, &principal())
        .await
        .unwrap();
    assert_eq!(guardrail.calls()[0].inputs.texts, Some(Vec::new()));
}

#[tokio::test]
async fn process_output_response_passes_the_response_and_key_metadata_in_the_request_data() {
    let guardrail = FakeGuardrail::passing_through();
    let mut subject =
        response(r#"{"model": "m", "choices": [{"index": 0, "message": {"content": "hi"}}]}"#);
    process_output_response(&mut subject, &guardrail, &principal())
        .await
        .unwrap();
    let request_data = &guardrail.calls()[0].request_data;
    assert_eq!(
        request_data.get("response"),
        Some(&GuardrailValue::Object(object(
            r#"{"model": "m", "choices": [{"index": 0, "message": {"content": "hi", "tool_calls": null}}]}"#
        )))
    );
    assert_eq!(
        request_data.get("metadata"),
        Some(&GuardrailValue::Object(object(
            r#"{"user_api_key_user_id": "user-1", "user_api_key_user_email": "user@example.com",
                "user_api_key_team_id": "team-1", "user_api_key_team_alias": "Platform",
                "user_api_key_org_id": "org-1", "user_api_key_end_user_id": "end-1",
                "user_api_key_hash": "hash-1"}"#
        )))
    );
}

#[tokio::test]
async fn process_output_response_keeps_the_text_when_the_guardrail_returns_none() {
    let guardrail = FakeGuardrail::answering(Box::new(|inputs| GuardrailApiInputs {
        texts: None,
        ..inputs
    }));
    let mut subject = response(TWO_CHOICES);
    process_output_response(&mut subject, &guardrail, &principal())
        .await
        .unwrap();
    assert_eq!(subject, response(TWO_CHOICES));
}

#[tokio::test]
async fn process_output_response_rewrites_the_tool_call_function() {
    let guardrail = FakeGuardrail::answering(Box::new(|inputs| GuardrailApiInputs {
        tool_calls: Some(vec![GuardrailValue::Object(object(
            r#"{"function": {"name": "renamed", "arguments": "{}"}}"#,
        ))]),
        ..inputs
    }));
    let mut subject = response(TWO_CHOICES);
    process_output_response(&mut subject, &guardrail, &principal())
        .await
        .unwrap();
    let expected = response(
        &TWO_CHOICES
            .replace("\"lookup\"", "\"renamed\"")
            .replace(r#"{\"q\":1}"#, "{}"),
    );
    assert_eq!(subject, expected);
}

#[tokio::test]
async fn process_output_response_ignores_a_tool_call_list_of_another_length() {
    let guardrail = FakeGuardrail::answering(Box::new(|inputs| GuardrailApiInputs {
        tool_calls: Some(Vec::new()),
        ..inputs
    }));
    let mut subject = response(TWO_CHOICES);
    process_output_response(&mut subject, &guardrail, &principal())
        .await
        .unwrap();
    assert_eq!(subject, response(TWO_CHOICES));
}

#[tokio::test]
async fn process_output_response_refuses_a_returned_tool_call_that_is_not_an_object() {
    let guardrail = FakeGuardrail::answering(Box::new(|inputs| GuardrailApiInputs {
        tool_calls: Some(vec![GuardrailValue::Text("x".to_owned())]),
        ..inputs
    }));
    let mut subject = response(TWO_CHOICES);
    let failure = process_output_response(&mut subject, &guardrail, &principal())
        .await
        .unwrap_err();
    assert!(matches!(failure, GuardrailFailure::Other(_)));
    assert_eq!(subject, response(TWO_CHOICES));
}

#[tokio::test]
async fn process_output_response_passes_a_guardrail_failure_on() {
    let blocked = GuardrailFailure::Http {
        status: 400,
        detail: "blocked".to_owned(),
    };
    let guardrail = FakeGuardrail::failing(blocked.clone());
    let mut subject = response(TWO_CHOICES);
    let failure = process_output_response(&mut subject, &guardrail, &principal())
        .await
        .unwrap_err();
    assert_eq!(failure, blocked);
    assert_eq!(subject, response(TWO_CHOICES));
}

#[tokio::test]
async fn process_output_response_adds_no_key_metadata_for_a_principal_without_identity() {
    let guardrail = FakeGuardrail::passing_through();
    let mut anonymous = principal();
    anonymous.user = None;
    anonymous.organization = None;
    anonymous.teams = Vec::new();
    anonymous.end_user = None;
    anonymous.credential_ref.token_id = None;
    let mut subject = response(r#"{"choices": [{"message": {"content": "hi"}}]}"#);
    process_output_response(&mut subject, &guardrail, &anonymous)
        .await
        .unwrap();
    assert_eq!(guardrail.calls()[0].request_data.get("metadata"), None);
}

#[test]
fn process_output_reads_a_null_index_as_zero_and_ignores_an_image_url_that_is_not_an_object() {
    let subject = response(
        r#"{"choices": [{"index": null, "message": {"content": [{"type": "image_url", "image_url": "https://i/1.png"}]}}]}"#,
    );
    assert_eq!(subject.choices[0].index, 0);
    assert!(extract_response(&subject).images.is_empty());
}
