#![no_main]

use betterlite_core::api::runway::tasks::record::decode_runway_task;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(task) = decode_runway_task(data) {
        drop(task.state());
        drop(task.created_at.value());
        drop(task.completed_at.value());
        drop(task.estimated_credits());
        drop(task.failure.or_default("unknown"));
        drop(task.failure_code.or_default("unknown"));
    }
});
