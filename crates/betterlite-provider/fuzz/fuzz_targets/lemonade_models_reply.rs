#![no_main]

use betterlite_provider::self_hosted_dispatch::lemonade::decode_model_names;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _names = decode_model_names(bytes);
});
