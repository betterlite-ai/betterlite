#![no_main]

use betterlite_bedrock::nova_embeddings::data_url::parse_nova_data_url;
use libfuzzer_sys::fuzz_target;
use std::str;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = str::from_utf8(bytes) else {
        return;
    };
    let _outcome = parse_nova_data_url(text);
});
