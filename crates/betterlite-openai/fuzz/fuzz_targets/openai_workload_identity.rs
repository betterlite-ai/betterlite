#![no_main]

use betterlite_openai::workload_identity::exchange::decode_token_exchange;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_token_exchange(bytes);
});
