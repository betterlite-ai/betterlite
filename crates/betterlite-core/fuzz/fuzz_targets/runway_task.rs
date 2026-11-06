#![no_main]

use betterlite_core::api::runway::tasks::record::decode_runway_task;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(task) = decode_runway_task(data) {
        drop(task.state());
    }
});
