#![no_main]

use betterlite_openai::code_execution_records::code_parsing::parse_code;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(arguments) = std::str::from_utf8(bytes) else {
        return;
    };
    let _code = parse_code(arguments);
});
