#![no_main]

use betterlite_bedrock::agentcore_search_reply::decode_agentcore_search_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: (&[u8], bool)| {
    let status = if input.1 { 500 } else { 200 };
    let headers = [(
        String::from("content-type"),
        String::from("text/event-stream"),
    )];
    drop(decode_agentcore_search_reply(input.0, status, &headers));
});
