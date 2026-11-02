#![no_main]
use betterlite_oracle::routing_scenarios::parse_scenario;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _scenario = parse_scenario(bytes);
});
