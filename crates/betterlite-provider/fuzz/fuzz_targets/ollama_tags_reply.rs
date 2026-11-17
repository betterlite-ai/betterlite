#![no_main]

use betterlite_provider::self_hosted_dispatch::ollama_models::decode_tags;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _tags = decode_tags(bytes);
});
