#![no_main]

use std::str;

use betterlite_vertex::provider_metadata::timestamp::LocalTimeContext;
use betterlite_vertex::vertex_files::records::{
    MetadataShape, decode_vertex_file_metadata, deleted_file,
};
use jiff::tz::TimeZone;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let local = LocalTimeContext {
        zone: TimeZone::UTC,
    };
    let _upload = decode_vertex_file_metadata(bytes, MetadataShape::Upload, &local);
    let _retrieve = decode_vertex_file_metadata(bytes, MetadataShape::Retrieve, &local);
    if let Ok(text) = str::from_utf8(bytes) {
        let _deleted = deleted_file(Some(text));
    }
});
