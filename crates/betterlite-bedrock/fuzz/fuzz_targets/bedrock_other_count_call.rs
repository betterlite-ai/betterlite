#![no_main]

use betterlite_bedrock::count_call::reply::decode_bedrock_count_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _result = decode_bedrock_count_reply(bytes);
});
