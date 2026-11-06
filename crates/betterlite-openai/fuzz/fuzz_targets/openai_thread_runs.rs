#![no_main]

use betterlite_openai::thread_runs::decode::decode_run_event;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _event = decode_run_event(bytes);
});
