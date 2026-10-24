#![no_main]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let source = String::from_utf8_lossy(bytes);
    let result = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&source);
    let _ = result;
});
