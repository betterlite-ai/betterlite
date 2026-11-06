#![cfg(test)]

use crate::case::{LoadedCase, load_cases};
use crate::helpers::{HelperRun, run_case};
use rstest::rstest;
use std::path::Path;

#[rstest]
#[case::submit("pydantic-ai-submit")]
#[case::polling("pydantic-ai-polling")]
#[case::poll_exhaustion("pydantic-ai-poll-exhaustion")]
#[case::no_task_id("pydantic-ai-no-task-id")]
#[case::failed_task("pydantic-ai-failed-task")]
#[case::failed_submit("pydantic-ai-failed-submit")]
#[case::streaming("pydantic-ai-streaming")]
#[case::transport_timeout("pydantic-ai-transport-timeout")]
#[case::none_removal("pydantic-ai-none-removal")]
fn pydantic_ai_provider(#[case] name: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("cases/a2a/{name}.yaml"));
    let cases = load_cases(&[path]).unwrap();
    let LoadedCase::Helper(case) = &cases[0] else {
        panic!("expected helper");
    };
    assert!(matches!(run_case(case, false).unwrap(), HelperRun::Match));
}
