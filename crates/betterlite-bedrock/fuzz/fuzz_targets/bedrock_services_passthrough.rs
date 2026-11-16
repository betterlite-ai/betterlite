#![no_main]
use betterlite_bedrock::passthrough::event::decode_passthrough_log_event;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    let _outcome = decode_passthrough_log_event(data);
});
