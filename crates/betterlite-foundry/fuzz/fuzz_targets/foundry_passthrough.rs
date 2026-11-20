#![no_main]

use betterlite_foundry::passthrough::ocr::decode_logged_ocr_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_logged_ocr_reply(
        None,
        "azure_ai/mistral-ocr-latest",
        "providers/mistral/azure/ocr",
        bytes,
    );
});
