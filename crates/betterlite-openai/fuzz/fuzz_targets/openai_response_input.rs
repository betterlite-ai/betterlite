#![no_main]

use betterlite_openai::response_input::schema::decode_response_tool_schema as decode_schema;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _schema = decode_schema(bytes);
});
