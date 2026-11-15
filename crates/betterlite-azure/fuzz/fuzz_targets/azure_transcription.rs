#![no_main]

use betterlite_azure::transcription::native::decode_native_transcription;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _reply = decode_native_transcription(200, &Default::default(), bytes);
});
