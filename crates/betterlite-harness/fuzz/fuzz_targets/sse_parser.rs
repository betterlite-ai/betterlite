#![no_main]

use betterlite_harness::endpoint::sse::SSEUsageParser;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|chunk: &[u8]| {
    let mut parser = SSEUsageParser::default();
    parser.feed(chunk);
    parser.close();
});
