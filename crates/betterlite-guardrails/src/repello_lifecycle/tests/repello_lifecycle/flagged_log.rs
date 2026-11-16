use std::collections::BTreeSet;

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::repello_lifecycle::hooks::HookTarget;
use betterlite_guardrails::repello_lifecycle::verdict::log_flagged_verdict;
use rstest::rstest;
use tracing::subscriber::set_default;

use crate::support::{
    CapturedLog, FLAGGED_REPLY, applied, guardrail, mock_repello, parsed, prompt_data, run_hook,
    settings,
};

#[rstest]
#[case::names_the_policy(FLAGGED_REPLY, "toxicity")]
#[case::no_policies(r#"{"verdict": "flagged"}"#, "None")]
fn a_flagged_verdict_logs_the_policies(#[case] reply: &str, #[case] shown: &str) {
    let captured = CapturedLog::default();
    let _guard = set_default(captured.subscriber(tracing::Level::WARN));
    log_flagged_verdict(&parsed(reply));
    let log = captured.text();
    assert!(
        log.contains("RepelloAI Argus flagged content (allowed):"),
        "{log}"
    );
    assert!(log.contains(shown), "{log}");
}

#[rstest]
#[case::passed(r#"{"verdict": "passed"}"#)]
#[case::blocked(r#"{"verdict": "blocked"}"#)]
#[case::absent("{}")]
fn a_verdict_other_than_flagged_logs_nothing(#[case] reply: &str) {
    let captured = CapturedLog::default();
    let _guard = set_default(captured.subscriber(tracing::Level::WARN));
    log_flagged_verdict(&parsed(reply));
    assert_eq!(captured.text(), "");
}

#[tokio::test]
async fn a_flagged_scan_allows_the_request_logs_it_and_records_success() {
    let captured = CapturedLog::default();
    let _guard = set_default(captured.subscriber(tracing::Level::WARN));
    let mock = mock_repello(StatusCode::OK, FLAGGED_REPLY).await;
    let run = run_hook(
        &guardrail(settings(&mock.base)),
        prompt_data(),
        HookTarget::Prompt,
        &BTreeSet::new(),
    )
    .await;
    run.result.unwrap();
    let log = captured.text();
    assert!(
        log.contains("RepelloAI Argus flagged content (allowed):"),
        "{log}"
    );
    assert!(log.contains("toxicity"), "{log}");
    assert_eq!(
        applied(&run.data),
        vec![GuardrailValue::Text(String::from("repello-test"))]
    );
}
