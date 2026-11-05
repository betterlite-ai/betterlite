#![no_main]
use betterlite_openai::compatible_line_stream::{Mode, decode_legacy_compatible_line};
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &str| {
    let _sync = decode_legacy_compatible_line(data, Mode::Sync);
    let _async = decode_legacy_compatible_line(data, Mode::Async);
});
