#![no_main]
use betterlite_server::mcp::transport_checks::peek::{
    has_top_level_method, initialize_client_info, is_initialize_request, is_jsonrpc_response,
    lossy_utf8,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _initialize = is_initialize_request(bytes);
    let _client = initialize_client_info(bytes);
    let _response = is_jsonrpc_response(bytes);
    let _method = has_top_level_method(&lossy_utf8(bytes));
});
