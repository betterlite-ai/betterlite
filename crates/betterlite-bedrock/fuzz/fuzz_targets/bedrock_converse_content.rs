#![no_main]

use betterlite_bedrock::converse_content::decode_converse_json_tool;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_converse_json_tool(bytes);
});
