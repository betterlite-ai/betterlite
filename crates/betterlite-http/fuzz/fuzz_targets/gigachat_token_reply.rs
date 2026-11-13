#![no_main]

use betterlite_http::gigachat_tokens::reply::decode_token_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _reply = decode_token_reply(bytes);
});
