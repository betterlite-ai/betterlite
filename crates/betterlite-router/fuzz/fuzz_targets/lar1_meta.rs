#![no_main]

use betterlite_router::lar1_routing::metadata::Lar1Metadata;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _outcome = Lar1Metadata::parse(data);
});
