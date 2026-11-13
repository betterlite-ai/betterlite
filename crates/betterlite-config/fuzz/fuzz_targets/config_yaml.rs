#![no_main]

use std::collections::BTreeMap;
use std::str;

use betterlite_config::loader::ConfigLoader;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(source) = str::from_utf8(bytes) else {
        return;
    };
    let _outcome = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source);
});
