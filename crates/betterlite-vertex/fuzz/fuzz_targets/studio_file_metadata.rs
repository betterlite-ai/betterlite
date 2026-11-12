#![no_main]

use betterlite_vertex::provider_metadata::timestamp::LocalTimeContext;
use betterlite_vertex::studio_files::metadata::{MetadataShape, decode_studio_file_metadata};
use betterlite_vertex::studio_files::urls::normalize_file_id;
use jiff::tz::TimeZone;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let local = LocalTimeContext {
        zone: TimeZone::UTC,
    };
    let _create = decode_studio_file_metadata(bytes, MetadataShape::Create, &local);
    let _retrieve = decode_studio_file_metadata(bytes, MetadataShape::Retrieve, &local);
    if let Ok(text) = std::str::from_utf8(bytes) {
        let _id = normalize_file_id(text);
    }
});
