#![cfg(test)]

use std::time::Duration;

use axum::http::StatusCode;
use rstest::rstest;

use super::shared::{NO_OVERRIDES, Trigger, analyzer, closed_settings, spawn_mock};
use crate::presidio_analysis::error::PresidioAnalysisError;

const CLOSED_PREFIX: &str = "Presidio analyzer returned invalid response; cannot verify PII when PII protection is configured: ";

#[rstest]
#[case::bad_request(StatusCode::BAD_REQUEST, "400")]
#[case::not_found(StatusCode::NOT_FOUND, "404")]
#[case::server_error(StatusCode::INTERNAL_SERVER_ERROR, "500")]
#[case::unavailable(StatusCode::SERVICE_UNAVAILABLE, "503")]
#[tokio::test]
async fn analyzer_http_error_fails_closed_with_the_status_and_a_body_sample(
    #[case] status: StatusCode,
    #[case] code: &str,
) {
    let mock = spawn_mock(
        move |_| (status, "text/plain", "analyzer is down".to_owned()),
        Duration::ZERO,
    )
    .await;
    let error = analyzer(closed_settings(&mock.base, Trigger::PiiEntities))
        .analyze_text("john", &NO_OVERRIDES)
        .await
        .unwrap_err();
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
        format!("{CLOSED_PREFIX}HTTP {code} from Presidio analyzer: analyzer is down")
    );
}

#[tokio::test]
async fn analyzer_http_error_cuts_the_body_sample_at_200_characters() {
    let mock = spawn_mock(
        |_| {
            (
                StatusCode::BAD_GATEWAY,
                "text/plain; charset=utf-8",
                "\u{e9}".repeat(300),
            )
        },
        Duration::ZERO,
    )
    .await;
    let error = analyzer(closed_settings(&mock.base, Trigger::OutputParsePii))
        .analyze_text("john", &NO_OVERRIDES)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "{CLOSED_PREFIX}HTTP 502 from Presidio analyzer: {}",
            "\u{e9}".repeat(200)
        )
    );
}

#[tokio::test]
async fn analyzer_http_error_keeps_a_short_body_whole() {
    let mock = spawn_mock(
        |_| (StatusCode::BAD_GATEWAY, "text/plain", "x".repeat(10)),
        Duration::ZERO,
    )
    .await;
    let error = analyzer(closed_settings(&mock.base, Trigger::ApplyToOutput))
        .analyze_text("john", &NO_OVERRIDES)
        .await
        .unwrap_err();
    assert!(error.to_string().ends_with(": xxxxxxxxxx"));
}
