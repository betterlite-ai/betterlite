#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_server::api::request_form::decode_request_form;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((length_bytes, rest)) = data.split_first_chunk::<2>() else {
        return;
    };
    let header_len = usize::from(u16::from_be_bytes(*length_bytes));
    let Some((header, body)) = rest.split_at_checked(header_len) else {
        return;
    };
    drop(decode_request_form(body, header, &ApiLimits::default()));
});
