#![no_main]

use betterlite_server::api::ocr_request::{RawOcrRequest, parse_ocr_request};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    let raw = RawOcrRequest {
        content_type: Some("application/json".to_owned()),
        body: body.to_vec(),
        header_req_format: None,
        cached_form: None,
    };
    let _outcome = parse_ocr_request(&raw);
});
