#![cfg(test)]

use std::path::Path;

use rstest::rstest;

use crate::case::{LoadedCase, load_cases};
use crate::helpers::{HelperRun, run_case};

fn case_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("cases/watsonx-orchestrate/{name}.yaml"))
}

fn run(name: &str) {
    let cases = load_cases(&[case_path(name)]).unwrap();
    let LoadedCase::Helper(case) = &cases[0] else {
        panic!("expected a helper case");
    };
    assert!(matches!(run_case(case, false).unwrap(), HelperRun::Match));
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
