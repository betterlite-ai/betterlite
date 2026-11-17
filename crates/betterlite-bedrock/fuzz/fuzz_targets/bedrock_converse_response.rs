#![no_main]

use betterlite_bedrock::converse_response::decode_converse_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_converse_reply(bytes);
});
