#![no_main]

use betterlite_config::replicate_price::decode_replicate_price;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = str::from_utf8(data) {
        let _price = decode_replicate_price(text);
    }
});
