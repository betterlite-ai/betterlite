#![no_main]

use betterlite_router::affinity_claims::pin::StoredPin;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _outcome = StoredPin::parse(data);
});
