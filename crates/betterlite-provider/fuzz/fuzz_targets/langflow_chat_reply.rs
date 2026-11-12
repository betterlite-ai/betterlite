#![no_main]

use betterlite_provider::langflow_chat::reply::decode::decode_content;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _content = decode_content(bytes, 200);
});
