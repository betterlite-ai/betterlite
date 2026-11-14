use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::noma_scan_policy::BLOCKED_MESSAGE;
use betterlite_guardrails::noma_v2::action::{self, Action};
use betterlite_guardrails::noma_v2::error::NomaV2Error;
use rstest::rstest;

use crate::support::{
    BLOCKED_REPLY, INTERVENED_REPLY, NONE_REPLY, field, guardrail, inputs, mock_noma, records,
    request_data, scan, settings, text_field, value,
};

#[rstest]
#[case::blocked(r#"{"action": "BLOCKED"}"#, Action::Blocked)]
#[case::none(r#"{"action": "NONE"}"#, Action::None)]
#[case::intervened(r#"{"action": "GUARDRAIL_INTERVENED"}"#, Action::GuardrailIntervened)]
fn parse_accepts_each_action(#[case] reply: &str, #[case] expected: Action) {
    assert_eq!(action::parse(&value(reply)).unwrap(), expected);
}

#[rstest]
#[case::unknown(r#"{"action": "MAYBE"}"#)]
#[case::missing(r#"{}"#)]
#[case::not_a_string(r#"{"action": 3}"#)]
#[case::not_an_object(r#"["NONE"]"#)]
fn parse_rejects_every_other_answer(#[case] reply: &str) {
    assert!(matches!(
        action::parse(&value(reply)),
        Err(NomaV2Error::InvalidAction)
    ));
}

#[tokio::test]
async fn a_none_action_returns_the_inputs_unchanged() {
    let mock = mock_noma(StatusCode::OK, NONE_REPLY).await;
    let outcome = scan(
        &guardrail(settings(&mock.base)),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    assert_eq!(outcome.result.unwrap(), inputs());
    assert_eq!(
        text_field(&records(&outcome.request_data)[0], "guardrail_status").as_deref(),
        Some("success")
    );
}

#[tokio::test]
async fn a_blocked_action_returns_the_blocked_message() {
    let mock = mock_noma(StatusCode::OK, BLOCKED_REPLY).await;
    let outcome = scan(
        &guardrail(settings(&mock.base)),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    let Err(GuardrailFailure::HttpObject { status, detail }) = outcome.result else {
        panic!("a BLOCKED action did not produce an HTTP object failure");
    };
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(String::from(BLOCKED_MESSAGE)))
    );
    assert_eq!(detail.get("details"), Some(&value(BLOCKED_REPLY)));
}

#[tokio::test]
async fn a_guardrail_intervened_action_replaces_the_lists() {
    let mock = mock_noma(StatusCode::OK, INTERVENED_REPLY).await;
    let outcome = scan(
        &guardrail(settings(&mock.base)),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    let updated = outcome.result.unwrap();
    assert_eq!(updated.texts, Some(vec![String::from("masked")]));
    assert_eq!(updated.images, Some(vec![String::from("aW1hZ2U=")]));
    let Some(tools) = &updated.tools else {
        panic!("the intervention did not replace the tools");
    };
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].tool_type, "function");
    assert_eq!(
        tools[0].extra.get("function"),
        Some(&value(r#"{"name": "lookup"}"#))
    );
    let Some(tool_calls) = &updated.tool_calls else {
        panic!("the intervention did not replace the tool calls");
    };
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0], value(r#"{"id": "call-2"}"#));
    assert_eq!(
        text_field(&records(&outcome.request_data)[0], "guardrail_status").as_deref(),
        Some("guardrail_intervened")
    );
}

#[tokio::test]
async fn an_intervention_leaves_a_field_that_is_not_a_list_alone() {
    let reply =
        r#"{"action": "GUARDRAIL_INTERVENED", "texts": ["masked"], "images": "not a list"}"#;
    let mock = mock_noma(StatusCode::OK, reply).await;
    let outcome = scan(
        &guardrail(settings(&mock.base)),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    let updated = outcome.result.unwrap();
    assert_eq!(updated.texts, Some(vec![String::from("masked")]));
    assert_eq!(updated.images, inputs().images);
}

#[tokio::test]
async fn an_intervention_with_a_wrong_element_type_fails_the_scan() {
    let reply = r#"{"action": "GUARDRAIL_INTERVENED", "texts": [7]}"#;
    let mock = mock_noma(StatusCode::OK, reply).await;
    let outcome = scan(
        &guardrail(settings(&mock.base)),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    let Err(GuardrailFailure::Other(message)) = outcome.result else {
        panic!("a wrongly typed intervention did not fail the scan");
    };
    assert_eq!(
        message,
        "the `texts` field of the Noma v2 intervention has the wrong shape"
    );
}

#[tokio::test]
async fn a_response_with_a_missing_action_fails_the_scan() {
    let mock = mock_noma(StatusCode::OK, "{}").await;
    let outcome = scan(
        &guardrail(settings(&mock.base)),
        inputs(),
        request_data(),
        GuardrailInputType::Request,
    )
    .await;
    let Err(GuardrailFailure::Other(message)) = outcome.result else {
        panic!("a response without an action did not fail the scan");
    };
    assert_eq!(message, "the Noma v2 response has no valid action");
    let record = &records(&outcome.request_data)[0];
    assert_eq!(
        field(record, "guardrail_response"),
        Some(&GuardrailValue::Text(String::from(
            "the Noma v2 response has no valid action"
        )))
    );
    assert_eq!(
        text_field(record, "guardrail_status").as_deref(),
        Some("guardrail_failed_to_respond")
    );
}
