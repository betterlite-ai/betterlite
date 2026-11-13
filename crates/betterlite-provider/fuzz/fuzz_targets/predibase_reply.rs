#![no_main]

use betterlite_provider::predibase::decode::decode_predibase_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _reply = decode_predibase_reply(bytes);
});
