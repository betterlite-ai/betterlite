#![no_main]

use betterlite_models::catalog::ModelCatalog;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _ = ModelCatalog::parse_remote(input);
});
