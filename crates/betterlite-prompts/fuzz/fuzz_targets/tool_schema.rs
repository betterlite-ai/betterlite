#![no_main]

use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::schema::flatten::flatten_and_drop;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 65_536 {
        return;
    }
    let Ok(schema) = serde_json::from_slice::<Object>(bytes) else {
        return;
    };
    drop(flatten_and_drop(&schema));
});
