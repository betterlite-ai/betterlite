#![cfg(test)]

use std::time::Duration;

use axum::http::StatusCode;
use rstest::rstest;

use super::shared::{NO_OVERRIDES, Trigger, analyzer, closed_settings, settings, spawn_mock};
use crate::presidio_analysis::error::PresidioAnalysisError;
use crate::presidio_analysis::wire::AnalyzeOutcome;

#[rstest]
#[case::html("text/html", "text/html")]
#[case::parameters_are_dropped_from_the_message("text/html; charset=utf-8", "text/html")]
#[case::mimetype_is_lowercased("Text/Plain", "text/plain")]
#[tokio::test]
async fn analyzer_non_json_fails_closed_naming_the_content_type(
    #[case] header: &'static str,
    #[case] reported: &str,
) {
    let mock = spawn_mock(
        move |_| (StatusCode::OK, header, "<html>gateway</html>".to_owned()),
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
    assert!(
        message.ends_with(&format!(
            "expected application/json Content-Type but received '{reported}'; body: '<html>gateway</html>'"
        )),
        "{message}"
    );
}

#[rstest]
#[case::charset("application/json; charset=utf-8")]
#[case::uppercase("Application/JSON")]
#[case::json_inside_a_longer_type("application/problem+json-application/json")]
#[tokio::test]
async fn analyzer_non_json_accepts_a_json_mimetype(#[case] header: &'static str) {
    let mock = spawn_mock(
        move |_| (StatusCode::OK, header, "[]".to_owned()),
        Duration::ZERO,
    )
    .await;
    let outcome = analyzer(settings(&mock.base))
        .analyze_text("john", &NO_OVERRIDES)
        .await
        .unwrap();
    assert!(matches!(outcome, AnalyzeOutcome::Items(found) if found.is_empty()));
}
