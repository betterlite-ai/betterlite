use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::workflow_runs::models::{WorkflowRunCreateRequest, WorkflowRunUpdateRequest};
use crate::workflow_runs::router::decode_body;

#[rstest]
fn replays_the_workflow_run_body_corpus(#[files("fuzz/corpus/workflow_run_body/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    drop(decode_body::<WorkflowRunCreateRequest>(&bytes));
    drop(decode_body::<WorkflowRunUpdateRequest>(&bytes));
}
