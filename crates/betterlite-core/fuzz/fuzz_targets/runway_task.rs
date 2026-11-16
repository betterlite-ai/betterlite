#![no_main]

use betterlite_core::api::runway::tasks::record::decode_runway_task;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(task) = decode_runway_task(data) {
        drop(task.state());
        drop(task.created_at.value().cloned());
        drop(task.completed_at.value().cloned());
        drop(task.estimated_credits().cloned());
        drop(task.progress.map(|share| share.to_string()));
        drop(task.failure.or_default("unknown"));
        drop(task.failure_code.or_default("unknown"));
    }
});
