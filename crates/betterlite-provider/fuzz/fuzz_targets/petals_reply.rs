#![no_main]

use betterlite_provider::petals::decode::decode_petals_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _reply = decode_petals_reply(bytes);
});
