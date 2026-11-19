#![no_main]

use std::str::from_utf8;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_server::passthrough::request_data::parse_request_by_content_type;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((length_bytes, rest)) = data.split_first_chunk::<2>() else {
        return;
    };
    let header_len = usize::from(u16::from_be_bytes(*length_bytes));
    let Some((header, body)) = rest.split_at_checked(header_len) else {
        return;
    };
    let Ok(content_type) = from_utf8(header) else {
        return;
    };
    drop(parse_request_by_content_type(
        content_type,
        body,
        &[],
        &ApiLimits::default(),
    ));
});
