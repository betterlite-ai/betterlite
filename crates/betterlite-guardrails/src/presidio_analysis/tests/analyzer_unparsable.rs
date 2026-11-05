#![cfg(test)]

use std::time::Duration;

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

#[tokio::test]
async fn analyzer_unparsable_treats_an_empty_json_body_as_unparsable() {
    let mock = spawn_mock(|_| json_reply(""), Duration::ZERO).await;
    let error = analyzer(settings(&mock.base))
        .analyze_text("john", &NO_OVERRIDES)
        .await
        .unwrap_err();
    assert!(matches!(error, PresidioAnalysisError::ResponseJson { .. }));
}
