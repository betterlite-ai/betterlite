#![no_main]

use std::str::from_utf8;

use betterlite_provider::opensandbox::helpers::sse::parse_lines;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = from_utf8(bytes) else {
        return;
    };
    let lines: Vec<String> = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_owned())
        .collect();
    let _result = parse_lines(&lines);
});
