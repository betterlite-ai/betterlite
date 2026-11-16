#![no_main]
use betterlite_server::mcp_rotation::blob::decode_mcp_rotation_blob;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _blob = decode_mcp_rotation_blob(bytes);
});
