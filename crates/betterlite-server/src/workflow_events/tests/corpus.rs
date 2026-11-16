use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::workflow_events::models::{WorkflowEventCreateRequest, WorkflowMessageCreateRequest};
use crate::workflow_runs::router::decode_body;

#[rstest]
fn replays_the_workflow_event_body_corpus(
    #[files("fuzz/corpus/workflow_event_body/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    drop(decode_body::<WorkflowEventCreateRequest>(&bytes));
    drop(decode_body::<WorkflowMessageCreateRequest>(&bytes));
}
