#![no_main]

use betterlite_lens::analysis::{Decision, Examined, FinalDecision, Record};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decision = Decision::model_validate_json(bytes);
    let _final = FinalDecision::model_validate_json(bytes);
    let _examined = Examined::model_validate_json(bytes);
});
