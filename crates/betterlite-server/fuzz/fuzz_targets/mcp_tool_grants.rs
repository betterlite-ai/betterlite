#![no_main]

use betterlite_server::mcp_permission_ids::decode::decode_mcp_tool_grants_json;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_mcp_tool_grants_json(bytes));
});
