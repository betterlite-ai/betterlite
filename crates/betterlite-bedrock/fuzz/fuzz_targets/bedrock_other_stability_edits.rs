#![no_main]

use betterlite_bedrock::stability_edits::reply::decode_stability_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _result = decode_stability_reply(bytes);
});
