#![no_main]

use betterlite_logging::otel::model::db_endpoint::parse_database_endpoint;
use libfuzzer_sys::fuzz_target;
use std::str;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(text) = str::from_utf8(bytes) {
        let _endpoint = parse_database_endpoint(Some(text));
    }
});
