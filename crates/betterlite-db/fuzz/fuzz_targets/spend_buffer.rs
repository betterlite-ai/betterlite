#![no_main]

use betterlite_db::redis_spend::codec::decode_tagged_spend_buffer;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _record = decode_tagged_spend_buffer(bytes);
});
