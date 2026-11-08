#![no_main]

use betterlite_mcp::request_headers::path_targets::target_names_from_path;
use libfuzzer_sys::fuzz_target;
use std::str;

fuzz_target!(|bytes: &[u8]| {
    let Ok(path) = str::from_utf8(bytes) else {
        return;
    };
    let _targets = target_names_from_path(path);
});
