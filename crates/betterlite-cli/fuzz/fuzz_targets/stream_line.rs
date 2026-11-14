#![no_main]

use betterlite_cli::commands::chat::stream_line::decode_line;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _event = decode_line(bytes);
});
