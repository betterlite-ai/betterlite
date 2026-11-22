#![no_main]

use betterlite_oracle::case::decode_case;
use betterlite_oracle::http::saml_idp::authn_request::decode_authn_request_id;
use libfuzzer_sys::fuzz_target;
use std::path::Path;

fuzz_target!(|bytes: &[u8]| {
    let _authn_request_id = decode_authn_request_id(bytes);
    let _case = decode_case(Path::new("<bytes>"), bytes);
});
