#![no_main]
use betterlite_oracle_runner::routing_mock_scripts::parse_script;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _script = parse_script(bytes);
});
