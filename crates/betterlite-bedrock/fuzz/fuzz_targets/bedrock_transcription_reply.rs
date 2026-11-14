#![no_main]

use betterlite_bedrock::transcription::reply::decode_transcription_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _result = decode_transcription_reply(bytes);
});
