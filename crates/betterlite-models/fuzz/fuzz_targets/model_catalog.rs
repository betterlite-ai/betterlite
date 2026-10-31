#![no_main]

use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _ = ModelCatalog::parse(input, UnknownFields::Warn);
});
