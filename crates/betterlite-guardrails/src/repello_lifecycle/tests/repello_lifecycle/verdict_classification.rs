use axum::http::StatusCode;
use betterlite_guardrails::repello_lifecycle::verdict::verdict_blocks;
use rstest::rstest;
use tracing::subscriber::set_default;

use crate::support::{
    BLOCKED_REPLY, CapturedLog, FLAGGED_REPLY, PASSED_REPLY, guardrail, mock_repello, parsed,
    prompt_data, records, run_pre_call, settings, text_field,
};

#[rstest]
#[case::blocked(r#"{"verdict": "blocked"}"#, true)]
#[case::passed(r#"{"verdict": "passed"}"#, false)]
#[case::flagged(r#"{"verdict": "flagged"}"#, false)]
#[case::unknown(r#"{"verdict": "maybe"}"#, true)]
#[case::empty(r#"{"verdict": ""}"#, true)]
#[case::absent(r#"{}"#, true)]
#[case::null(r#"{"verdict": null}"#, true)]
fn only_passed_and_flagged_verdicts_allow_the_request(#[case] reply: &str, #[case] blocks: bool) {
    let response = parsed(reply);
    assert_eq!(verdict_blocks(Some(&response)), blocks);
}

#[test]
fn no_response_never_blocks() {
    assert!(!verdict_blocks(None));
}

#[rstest]
#[case::unknown(r#"{"verdict": "maybe"}"#, "maybe")]
#[case::absent(r#"{}"#, "None")]
fn an_unrecognized_verdict_warns_and_names_the_value(#[case] reply: &str, #[case] shown: &str) {
    let captured = CapturedLog::default();
    let _guard = set_default(captured.subscriber(tracing::Level::WARN));
    let response = parsed(reply);
    assert!(verdict_blocks(Some(&response)));
    let log = captured.text();
    assert!(
        log.contains("RepelloAI Argus returned an unrecognized verdict"),
        "{log}"
    );
    assert!(log.contains(shown), "{log}");
    assert!(log.contains("- blocking."), "{log}");
}

#[rstest]
#[case::blocked(BLOCKED_REPLY, "guardrail_intervened", false)]
#[case::passed(PASSED_REPLY, "success", true)]
#[case::flagged(FLAGGED_REPLY, "success", true)]
#[tokio::test]
async fn the_scan_records_the_verdict_status(
    #[case] reply: &str,
    #[case] status: &str,
    #[case] allowed: bool,
) {
    let mock = mock_repello(StatusCode::OK, reply).await;
    let run = run_pre_call(&guardrail(settings(&mock.base)), prompt_data()).await;
    assert_eq!(run.result.is_ok(), allowed);
    let recorded = records(&run.data);
    assert_eq!(recorded.len(), 1);
    assert_eq!(
        text_field(&recorded[0], "guardrail_status").as_deref(),
        Some(status)
    );
}

#[tokio::test]
async fn an_unknown_verdict_blocks_the_request() {
    let mock = mock_repello(StatusCode::OK, r#"{"verdict": "quarantined"}"#).await;
    let run = run_pre_call(&guardrail(settings(&mock.base)), prompt_data()).await;
    let Err(error) = run.result else {
        panic!("an unknown verdict must block");
    };
    assert_eq!(error.to_string(), "Blocked by RepelloAI Argus guardrail.");
}
