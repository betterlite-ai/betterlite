#![no_main]

use betterlite_bedrock::bedrock_embedding_io::decode::decode_embedding_response;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_embedding_response(bytes);
});
