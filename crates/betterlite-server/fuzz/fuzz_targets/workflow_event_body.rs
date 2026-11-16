#![no_main]

use betterlite_server::workflow_events::models::{
    WorkflowEventCreateRequest, WorkflowMessageCreateRequest,
};
use betterlite_server::workflow_runs::router::decode_body;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    drop(decode_body::<WorkflowEventCreateRequest>(body));
    drop(decode_body::<WorkflowMessageCreateRequest>(body));
});
