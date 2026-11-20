#![no_main]

use betterlite_provider::api::realtime_http_transport::decode::decode_realtime_sdp;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_realtime_sdp(bytes, Default::default());
});
