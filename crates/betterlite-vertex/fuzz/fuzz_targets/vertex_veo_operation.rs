#![no_main]

use betterlite_vertex::vertex_veo_operation::content::content_response;
use betterlite_vertex::vertex_veo_operation::decode::decode_vertex_veo_operation;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_vertex_veo_operation(bytes));
    drop(content_response(bytes));
});
