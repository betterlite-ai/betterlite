#![no_main]

use betterlite_bedrock::vector_stores::response::decode_knowledge_base_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_knowledge_base_reply(200, &[], bytes, "");
});
