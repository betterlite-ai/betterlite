#![no_main]
use std::collections::BTreeMap;
use std::str::from_utf8;

use betterlite_logging::datadog_logging::settings::{DD_MAX_BATCH_SIZE, resolve_batch_size};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = from_utf8(bytes) else {
        return;
    };
    let environment = BTreeMap::from([("DD_BATCH_SIZE".to_owned(), text.to_owned())]);
    let resolved = resolve_batch_size(&environment);
    assert!((1..=DD_MAX_BATCH_SIZE).contains(&resolved));
});
