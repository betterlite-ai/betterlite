#![no_main]

use betterlite_azure::search::reply::{AzureSearchReply, decode_grounding_reply};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let reply = AzureSearchReply {
        status_code: 200,
        headers: &[],
        body: bytes,
    };
    let _decoded = decode_grounding_reply(&reply);
});
