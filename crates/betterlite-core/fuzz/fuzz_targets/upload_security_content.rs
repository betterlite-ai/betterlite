#![no_main]

use betterlite_core::rag::upload_security::{ContentInspection, DetectedFormat, inspect_content};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() <= 0x0001_0000
        && let ContentInspection::Allowed(allowed) = inspect_content(data)
    {
        match allowed.format {
            DetectedFormat::Pdf => assert!(data.starts_with(b"%PDF-")),
            DetectedFormat::Text => {
                assert!(!data.contains(&0));
                assert_eq!(str::from_utf8(data).map(str::as_bytes), Ok(data));
            }
        }
    }
});
