#![no_main]

use betterlite_vertex::provider_metadata::timestamp::{LocalTimeContext, decode_vertex_timestamp};
use jiff::tz::TimeZone;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let context = LocalTimeContext {
        zone: TimeZone::UTC,
    };
    let _decoded = decode_vertex_timestamp(bytes, &context);
});
