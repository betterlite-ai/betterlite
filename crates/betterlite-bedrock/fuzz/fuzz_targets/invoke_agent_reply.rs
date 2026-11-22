#![no_main]

use betterlite_bedrock::invoke_agent_reply::decode_agent_trace;
use betterlite_bedrock::invoke_agent_reply::extract::{response_content, usage_info};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(events) = decode_agent_trace(bytes) {
        drop(response_content(&events));
        drop(usage_info(&events));
    }
});
