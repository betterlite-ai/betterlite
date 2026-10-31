#![no_main]

use betterlite_db::mcp_stored_contracts::decode::decode_mcp_stored_contract;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = decode_mcp_stored_contract(bytes);
});
