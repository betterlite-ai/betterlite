#![no_main]

use betterlite_db::tool_records::decode::decode_tool_record;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_tool_record(bytes);
});
