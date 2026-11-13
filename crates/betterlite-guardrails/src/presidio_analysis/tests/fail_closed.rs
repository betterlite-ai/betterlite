#![cfg(test)]

use std::time::Duration;

use rstest::rstest;

use super::shared::{NO_OVERRIDES, Trigger, analyzer, closed_settings, json_reply, spawn_mock};
use crate::presidio_analysis::error::PresidioAnalysisError;

#[rstest]
#[case::pii_entities(Trigger::PiiEntities)]
#[case::output_parse_pii(Trigger::OutputParsePii)]
#[case::apply_to_output(Trigger::ApplyToOutput)]
#[tokio::test]
async fn fail_closed_raises_a_guardrail_error_for_an_error_dict(#[case] trigger: Trigger) {
    let mock = spawn_mock(
        |_| json_reply(r#"{"error":"analyzer exploded"}"#),
        Duration::ZERO,
    )
    .await;
    let error = analyzer(closed_settings(&mock.base, trigger))
        .analyze_text("john", &NO_OVERRIDES)
        .await
        .unwrap_err();
    assert_eq!(error.status(), 400);
    let PresidioAnalysisError::GuardrailRaised {
        guardrail_name,
        message,
    } = error
    else {
        panic!("expected a guardrail error");
    };
    assert_eq!(guardrail_name, "presidio-guard");
    assert_eq!(
        message,
        "Presidio analyzer returned invalid response; cannot verify PII when PII protection is configured: error: \"analyzer exploded\""
    );
}

#[rstest]
#[case::string_body(
    "\"oops\"",
    "unexpected type string (expected list or dict), response: \"oops\""
)]
#[case::number_body("42", "unexpected type integer (expected list or dict), response: 42")]
#[case::null_body("null", "unexpected type null (expected list or dict), response: null")]
#[case::bool_body(
    "true",
    "unexpected type boolean (expected list or dict), response: true"
)]
#[case::float_body("1.5", "unexpected type float (expected list or dict), response: 1.5")]
#[case::dict_that_is_not_an_item(r#"{"entity_type":5}"#, "failed to parse dict response")]
#[tokio::test]
async fn fail_closed_names_what_was_wrong_with_the_response(
    #[case] body: &'static str,
    #[case] reason: &str,
) {
    let mock = spawn_mock(|request| json_reply(&request.text), Duration::ZERO).await;
    let error = analyzer(closed_settings(&mock.base, Trigger::PiiEntities))
        .analyze_text(body, &NO_OVERRIDES)
        .await
        .unwrap_err();
    let PresidioAnalysisError::GuardrailRaised { message, .. } = error else {
        panic!("expected a guardrail error");
    };
    assert!(message.ends_with(reason), "{message}");
}
