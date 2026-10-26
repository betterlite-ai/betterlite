#![no_main]

use betterlite_lens::analysis::{Decision, Examined};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decision = serde_json::from_slice::<Decision>(bytes);
    let _examined = serde_json::from_slice::<Examined>(bytes);
});
