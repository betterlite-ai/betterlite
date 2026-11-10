#![no_main]
use betterlite_openai::gigachat_stream::chunk_parser;
use betterlite_openai::gigachat_utils::UuidIds;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let _parsed = chunk_parser(bytes, &UuidIds);
});
