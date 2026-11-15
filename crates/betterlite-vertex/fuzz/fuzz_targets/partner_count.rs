#![no_main]

use betterlite_vertex::partner_count::response::decode_partner_count;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_partner_count(bytes));
});
