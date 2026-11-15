#![no_main]
use betterlite_server::mcp_rotation::blob::{
    decode_json_column, decode_legacy_base64_text, decode_mcp_rotation_blob,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _blob = decode_mcp_rotation_blob(bytes);
    if let Ok(text) = std::str::from_utf8(bytes) {
        let _column = decode_json_column(Some(text));
        let _legacy = decode_legacy_base64_text(text);
    }
});
