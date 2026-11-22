#![no_main]

use betterlite_server::search_tool_routes::body::decode_body;
use betterlite_server::search_tool_routes::connection::ConnectionBody;
use betterlite_server::search_tool_routes::records::WriteBody;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    let _write = decode_body::<WriteBody>(body);
    let _connection = decode_body::<ConnectionBody>(body);
});
