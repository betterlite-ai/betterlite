#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &str| {
    let _ = betterlite_openai::compatible_line_stream::decode_legacy_compatible_line(
        data,
        betterlite_openai::compatible_line_stream::Mode::Async,
    );
});
