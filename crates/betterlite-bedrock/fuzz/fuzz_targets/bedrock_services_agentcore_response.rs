#![no_main]

use betterlite_bedrock::agentcore_response::reply::decode_agentcore_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: (&[u8], bool)| {
    let content_type = if input.1 {
        Some("application/json")
    } else {
        Some("text/event-stream")
    };
    drop(decode_agentcore_reply(input.0, content_type));
});
