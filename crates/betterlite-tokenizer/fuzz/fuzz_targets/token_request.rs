#![no_main]

use betterlite_tokenizer::count::wire::TokenRequestWire;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _request = TokenRequestWire::try_from(bytes);
});
