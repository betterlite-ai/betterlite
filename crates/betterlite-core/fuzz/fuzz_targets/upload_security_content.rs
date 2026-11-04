#![no_main]

use betterlite_core::rag::upload_security::inspect_content;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() <= 65_536 {
        let _ = inspect_content(data);
    }
});
