#![no_main]

use betterlite_provider::billing::decode_openai_costs_page;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_openai_costs_page(bytes);
});
