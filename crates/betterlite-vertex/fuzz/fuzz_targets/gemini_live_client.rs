#![no_main]

// ast-grep-ignore: no-long-names
use betterlite_vertex::gemini_live_client::decode::decode_gemini_live_client_event;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    // ast-grep-ignore: no-long-names
    drop(decode_gemini_live_client_event(bytes, usize::MAX));
});
