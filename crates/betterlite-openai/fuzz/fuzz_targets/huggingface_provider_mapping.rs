#![no_main]
use betterlite_openai::huggingface_chat::mapping::decode_provider_mapping;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    let _mapping = decode_provider_mapping(data);
});
