#![no_main]

use betterlite_vertex::provider_metadata::timestamp::LocalTimeContext;
use betterlite_vertex::studio_file_search::ResponseHeaders;
use betterlite_vertex::studio_file_search::decode::{
    RawResponse, Shape, decode_studio_file_search,
};
use jiff::tz::TimeZone;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let headers = ResponseHeaders::new();
    let response = RawResponse {
        status: 200,
        headers: &headers,
        body: bytes,
    };
    let local = LocalTimeContext {
        zone: TimeZone::UTC,
    };
    let _search = decode_studio_file_search(Shape::Search, &response, "query", &local);
    let _create = decode_studio_file_search(Shape::Create, &response, "", &local);
});
