#![no_main]

use betterlite_bedrock::invoke_events::framing::decode_bedrock_event_json;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_bedrock_event_json(bytes);
});
