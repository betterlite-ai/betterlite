#![no_main]

use betterlite_openai::openai_video_identifiers::ids::decode_provider_video_id;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_provider_video_id(bytes);
});
