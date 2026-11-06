#![no_main]

use betterlite_guardrails::prisma_airs::inputs::Inputs;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _inputs = serde_json::from_slice::<Inputs>(bytes);
});
