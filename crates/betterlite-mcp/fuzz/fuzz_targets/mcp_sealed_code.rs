#![no_main]

use betterlite_mcp::oauth_state::SealCipher;
use betterlite_mcp::oauth_state::sealed::{open_bridge_code, open_passthrough_code};
use libfuzzer_sys::fuzz_target;

const MARKER: &str = "fuzz:";

// A fixed cipher so the fuzzer reaches both openers' JSON parsing and field checks without a master key.
struct FuzzCipher;

impl SealCipher for FuzzCipher {
    fn encrypt(&self, plain: &str) -> String {
        format!("{MARKER}{plain}")
    }

    fn decrypt(&self, value: &str, _field: &str) -> Option<String> {
        value.strip_prefix(MARKER).map(str::to_owned)
    }
}

fuzz_target!(|bytes: &[u8]| {
    let code = lossy_text(bytes);
    let _bridge = open_bridge_code(&code, &FuzzCipher);
    let _passthrough = open_passthrough_code(&code, &FuzzCipher);
});

// the code form field reaches the openers as text, so invalid UTF-8 is replaced rather than skipped
fn lossy_text(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push(char::REPLACEMENT_CHARACTER);
        }
    }
    text
}
