#![no_main]

use betterlite_bedrock::rerank::decode::decode_rerank_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _result = decode_rerank_reply(bytes, "generated-id");
});
