#![no_main]

use betterlite_vertex::studio_veo_operation::decode::decode_studio_veo_operation;
use betterlite_vertex::studio_veo_operation::download::download_target;
use betterlite_vertex::studio_veo_operation::records::status_response;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_studio_veo_operation(bytes));
    drop(status_response(bytes, Some("gemini")));
    drop(download_target(bytes));
});
