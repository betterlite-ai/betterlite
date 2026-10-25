#![no_main]

use betterlite_anthropic::cache_plan::parse_cache_plan;
use libfuzzer_sys::fuzz_target;
use serde_json::Value;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(Value::Object(body)) = serde_json::from_slice(bytes) {
        let _ = parse_cache_plan(&body);
    }
});
