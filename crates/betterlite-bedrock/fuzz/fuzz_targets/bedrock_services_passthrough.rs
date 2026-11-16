#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = betterlite_bedrock::passthrough::event::decode_passthrough_log_event(data);
});
