#![cfg(test)]

use std::path::{Path, PathBuf};

use rstest::rstest;

use betterlite_oracle_runner::case::load_helper_cases;
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};

fn case_path(name: &str) -> PathBuf {
    Path::new(ORACLE_DIR).join(format!("cases/watsonx-orchestrate/{name}.yaml"))
}

fn run(name: &str) {
    let cases = load_helper_cases(&[case_path(name)]).unwrap();
    let case = &cases[0];
    assert!(matches!(
        run_case(&[crate::GROUP], case, false).unwrap(),
        HelperRun::Match
    ));
}

#[rstest]
#[case::run_submit_non_streaming("run-submit-non-streaming")]
#[case::run_poll_success("run-poll-success")]
#[case::run_poll_timeout("run-poll-timeout")]
#[case::run_poll_terminal_failed("run-poll-terminal-failed")]
fn watsonx_run_cases(#[case] name: &str) {
    run(name);
}

#[rstest]
#[case::run_submit_streaming("run-submit-streaming")]
#[case::stream_sse_content("stream-sse-content")]
#[case::stream_json_content_poll("stream-json-content-poll")]
#[case::stream_fallback_transport_error("stream-fallback-transport-error")]
#[case::stream_fallback_not_triggered_on_status_error(
    "stream-fallback-not-triggered-on-status-error"
)]
#[case::stream_timeout("stream-timeout")]
fn watsonx_stream_cases(#[case] name: &str) {
    run(name);
}
