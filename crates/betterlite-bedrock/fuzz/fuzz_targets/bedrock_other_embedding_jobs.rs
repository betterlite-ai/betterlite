#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = betterlite_bedrock::embedding_jobs::decode::decode_async_embedding_status(bytes);
});
