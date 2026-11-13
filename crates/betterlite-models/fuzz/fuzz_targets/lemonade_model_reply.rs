#![no_main]

use betterlite_models::lemonade_model_metadata::decode_lemonade_model_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_lemonade_model_reply(bytes);
});
