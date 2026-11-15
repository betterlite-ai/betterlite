#![no_main]

use betterlite_vertex::provider_metadata::timestamp::LocalTimeContext;
use betterlite_vertex::vertex_fine_tuning::records::response::decode_vertex_tuning_job;
use jiff::tz::TimeZone;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let local = LocalTimeContext {
        zone: TimeZone::UTC,
    };
    let _decoded = decode_vertex_tuning_job(bytes, &local);
});
