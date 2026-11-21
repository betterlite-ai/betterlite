#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::file_http_upload::decoder::decode_file_upload_location;
use betterlite_openai::api::file_type_records::UploadUrlLocation;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let location = match bytes.first() {
        Some(0) => UploadUrlLocation::Headers,
        _ => UploadUrlLocation::Body,
    };
    let body = bytes.get(1..).unwrap_or_default();
    drop(decode_file_upload_location(
        body,
        location,
        "upload_url",
        ApiLimits::default(),
    ));
});
