#![no_main]

use betterlite_guardrails::vigil_guard::analysis::Analysis;
use betterlite_guardrails::vigil_guard::output::Inputs;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _analysis = serde_json::from_slice::<Analysis>(bytes);
    let _inputs = serde_json::from_slice::<Inputs>(bytes);
});
