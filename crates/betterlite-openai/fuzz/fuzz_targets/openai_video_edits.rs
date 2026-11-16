#![no_main]

use betterlite_openai::video_edits::response::decode_character_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_character_reply(bytes);
});
