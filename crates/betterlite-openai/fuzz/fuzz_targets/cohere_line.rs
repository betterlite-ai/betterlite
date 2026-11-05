#![no_main]
use betterlite_openai::cohere_streams::{CohereStreamVersion, decode_cohere_line};
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let _v1 = decode_cohere_line(CohereStreamVersion::V1, bytes, true);
    let _v2 = decode_cohere_line(CohereStreamVersion::V2, bytes, true);
});
