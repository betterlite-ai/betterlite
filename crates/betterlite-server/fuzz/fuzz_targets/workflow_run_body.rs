#![no_main]

use betterlite_server::workflow_runs::models::{
    WorkflowRunCreateRequest, WorkflowRunUpdateRequest,
};
use betterlite_server::workflow_runs::router::decode_body;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    drop(decode_body::<WorkflowRunCreateRequest>(body));
    drop(decode_body::<WorkflowRunUpdateRequest>(body));
});
