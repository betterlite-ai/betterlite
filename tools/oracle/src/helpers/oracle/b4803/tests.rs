#![cfg(test)]

use crate::case::{LoadedCase, load_cases};
use crate::helpers::{HelperRun, run_case};
use rstest::rstest;
use std::path::Path;

#[rstest]
#[case::extract("message-text-extraction")]
#[case::metadata("metadata-forward")]
#[case::role("role-passthrough")]
#[case::response("openai-to-a2a-response")]
#[case::task("task-event")]
#[case::working("status-working-event")]
#[case::completed("status-completed-event")]
#[case::artifact("artifact-event")]
#[case::parts_empty("parts-empty")]
#[case::parts_null_kind("parts-null-kind")]
#[case::metadata_absent("metadata-absent")]
#[case::metadata_empty("metadata-empty")]
#[case::metadata_params("metadata-params")]
#[case::metadata_invalid("metadata-invalid")]
#[case::metadata_body_invalid("metadata-body-invalid")]
#[case::role_default("role-default")]
#[case::role_agent("role-agent")]
#[case::response_empty("response-empty")]
#[case::response_null_content("response-null-content")]
#[case::task_default("task-default")]
fn a2a_completion_bridge(#[case] name: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("cases/a2a/{name}.yaml"));
    let cases = load_cases(&[path]).unwrap();
    let LoadedCase::Helper(case) = &cases[0] else {
        panic!("expected helper");
    };
    assert!(matches!(run_case(case, false).unwrap(), HelperRun::Match));
}
