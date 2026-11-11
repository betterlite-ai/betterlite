#![no_main]

use betterlite_models::judge_primitives::judge_verdict_parsing::parse_json_verdict;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|value: &str| {
    let _verdict = parse_json_verdict(value);
});
