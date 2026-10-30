#![no_main]

use betterlite_core::rag::document::extract_text_from_pdf;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _extracted = extract_text_from_pdf(bytes);
});
