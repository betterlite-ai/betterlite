#![no_main]

use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::sambanova_embedding::decode_sambanova_embedding;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _reply = decode_sambanova_embedding(bytes, 502, &Headers::new());
});
