#![no_main]
use betterlite_config::sections::mcp_tools::decode_mcp_tools;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let _section = decode_mcp_tools(bytes);
});
