#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _message = betterlite_server::mcp::stream_transport::wire::decode_message(data);
});
