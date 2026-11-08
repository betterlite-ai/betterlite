#![no_main]

use betterlite_mcp::request_headers::path_targets::target_names_from_path;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(path) = std::str::from_utf8(bytes) {
        let _targets = target_names_from_path(path);
    }
});
