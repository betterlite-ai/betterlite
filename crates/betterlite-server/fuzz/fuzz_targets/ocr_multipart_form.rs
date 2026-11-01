#![no_main]

use betterlite_server::api::ocr_request::{RawOcrRequest, parse_ocr_request};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    if let Ok(runtime) = tokio::runtime::Builder::new_current_thread().build() {
        runtime.block_on(async {
            let raw = RawOcrRequest {
                content_type: Some("multipart/form-data; boundary=FUZZ".to_owned()),
                body: body.to_vec(),
                header_req_format: None,
                cached_form: None,
            };
            let _ = parse_ocr_request(&raw).await;
        });
    }
});
