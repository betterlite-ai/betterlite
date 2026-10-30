#![no_main]

use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::definitions::expand::unpack_defs;
use betterlite_prompts::definitions::legacy::{DEFS_MAX_INLINED_BYTES, unpack_legacy_defs};
use betterlite_prompts::definitions::sanitize::sanitize_for_anthropic;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 65_536 {
        return;
    }
    let Ok(schema) = serde_json::from_slice::<Object>(bytes) else {
        return;
    };
    drop(unpack_defs(
        schema.clone(),
        &Object::new(),
        Some(DEFS_MAX_INLINED_BYTES),
    ));
    drop(unpack_legacy_defs(schema.clone(), DEFS_MAX_INLINED_BYTES));
    drop(sanitize_for_anthropic(schema));
});
