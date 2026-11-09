#![no_main]
use betterlite_server::mcp::stream_transport::wire::decode_message;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _message = decode_message(bytes);
});
