#![no_main]

use std::collections::BTreeMap;

use betterlite_azure::transcription::native::reply::decode_native_transcription;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _reply = decode_native_transcription(200, &BTreeMap::new(), bytes);
});
