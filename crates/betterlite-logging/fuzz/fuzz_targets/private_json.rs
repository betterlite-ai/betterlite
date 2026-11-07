#![no_main]

use betterlite_logging::private_json::safe_json::{
    DEFAULT_MAX_RECURSE_DEPTH, safe_dumps, safe_json_loads,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(value) = safe_json_loads(bytes) {
        let _dumped = safe_dumps(&value, DEFAULT_MAX_RECURSE_DEPTH, None);
    }
});
