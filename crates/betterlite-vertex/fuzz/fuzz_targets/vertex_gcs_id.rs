#![no_main]

use betterlite_vertex::vertex_gcs_access::decode::decode_vertex_gcs_id;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_vertex_gcs_id(bytes, "bucket/litellm-vertex-files", None);
});
