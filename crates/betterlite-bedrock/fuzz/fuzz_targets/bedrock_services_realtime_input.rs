#![no_main]
use betterlite_bedrock::realtime_input::decode::decode_realtime_client_input;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    let _outcome = decode_realtime_client_input(data);
});
