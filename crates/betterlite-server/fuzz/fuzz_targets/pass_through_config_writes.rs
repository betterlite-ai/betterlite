#![no_main]

use betterlite_server::endpoints::passthrough_config_writes::routes::decode_body;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    drop(decode_body(body));
});
