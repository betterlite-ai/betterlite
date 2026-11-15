#![no_main]

use betterlite_openai::videos::reply::decode_video_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _reply = decode_video_reply(bytes);
});
