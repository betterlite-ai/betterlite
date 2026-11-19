#![no_main]

use betterlite_db::autorouter_facts::turn_time::decode_turn_time;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_turn_time(bytes);
});
