#![no_main]

use betterlite_azure::azure_stream_errors::content_filter::decode_content_filter_error;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_content_filter_error(bytes);
});
