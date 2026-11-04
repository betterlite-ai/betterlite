#![no_main]

use betterlite_prompts::schema::{drop_non_python_regex_patterns, flatten::flatten_top_level};

use betterlite_prompts::arguments::json::Object;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 65_536 {
        return;
    }
    let Ok(schema) = serde_json::from_slice::<Object>(bytes) else {
        return;
    };
    drop(flatten_top_level(&flatten_top_level(
        &drop_non_python_regex_patterns(&schema),
    )));
});
