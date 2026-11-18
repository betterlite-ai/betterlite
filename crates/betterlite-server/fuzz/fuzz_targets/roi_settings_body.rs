#![no_main]

use betterlite_server::roi_settings::route::parse_update_body;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    drop(parse_update_body(body));
});
