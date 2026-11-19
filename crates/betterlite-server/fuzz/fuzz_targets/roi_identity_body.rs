#![no_main]

use betterlite_server::roi_routes::identity::parse_identity_body;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    drop(parse_identity_body(body));
});
