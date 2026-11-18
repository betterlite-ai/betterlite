#![no_main]

use betterlite_bedrock::embedding_jobs::decode::decode_async_embedding_status;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_async_embedding_status(bytes);
});
