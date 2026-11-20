#![no_main]

use betterlite_server::compliance_checks::decode::decode_compliance_request;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_compliance_request(bytes));
});
