#![no_main]

use betterlite_server::shadow_eval_validation::request::parse_shadow_eval_request;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = parse_shadow_eval_request(bytes);
});
