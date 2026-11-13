#![no_main]

use betterlite_models::endpoints::{RegistryAsset, parse};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _root = parse(input, RegistryAsset::Root);
    let _packaged = parse(input, RegistryAsset::Packaged);
});
