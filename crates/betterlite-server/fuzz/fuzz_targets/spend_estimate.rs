#![no_main]

use betterlite_server::spend::estimates::decode::decode_spend_estimate;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    drop(decode_spend_estimate(body));
});
