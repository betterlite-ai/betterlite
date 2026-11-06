#![cfg(test)]

use std::time::Duration;

use axum::http::StatusCode;
use rstest::rstest;

use super::shared::{
    NO_OVERRIDES, Trigger, analyzer, closed_settings, json_reply, settings, spawn_mock,
};
use crate::presidio_analysis::error::PresidioAnalysisError;

#[rstest]
#[case::fail_closed(Some(Trigger::PiiEntities))]
#[case::fail_open(None)]
#[tokio::test]
async fn analyzer_unparsable_is_an_internal_error_in_both_modes(#[case] mode: Option<Trigger>) {
    let mock = spawn_mock(|_| json_reply("not json at all"), Duration::ZERO).await;
    let configured = match mode {
        Some(trigger) => closed_settings(&mock.base, trigger),
        None => settings(&mock.base),
    };
    let error = analyzer(configured)
        .analyze_text("john", &NO_OVERRIDES)
        .await
        .unwrap_err();
    assert!(matches!(error, PresidioAnalysisError::ResponseJson { .. }));
    assert_eq!(error.status(), 500);
    assert_eq!(
        error.to_string(),
        "Presidio PII analysis failed: analyzer response is not JSON"
    );
}

#[rstest]
#[case::empty("")]
#[case::whitespace_only(" \n\t ")]
#[tokio::test]
async fn analyzer_unparsable_treats_a_blank_body_as_an_unexpected_type(#[case] body: &'static str) {
    let mock = spawn_mock(
        move |_| (StatusCode::OK, "application/json", body.to_owned()),
        Duration::ZERO,
    )
    .await;
    let error = analyzer(closed_settings(&mock.base, Trigger::PiiEntities))
        .analyze_text("john", &NO_OVERRIDES)
        .await
        .unwrap_err();
    let PresidioAnalysisError::GuardrailRaised { message, .. } = error else {
        panic!("expected a guardrail error");
    };
    assert!(message.contains("unexpected type null"), "{message}");
}
