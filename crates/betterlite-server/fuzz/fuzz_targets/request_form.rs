#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_server::api::request_form::decode_request_form;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 2 {
        return;
    }
    let header_len = usize::from(u16::from_be_bytes([data[0], data[1]]));
    let Some(rest) = data.get(2..) else {
        return;
    };
    if header_len > rest.len() {
        return;
    }
    let (header, body) = rest.split_at(header_len);
    let _ = decode_request_form(body, header, &ApiLimits::default());
});
