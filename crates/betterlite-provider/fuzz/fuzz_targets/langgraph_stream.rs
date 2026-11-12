#![no_main]

use betterlite_openai::aleph_alpha::lines::decode_text;
use betterlite_provider::langgraph_stream::UuidV4;
use betterlite_provider::langgraph_stream::iterator::parse_sse_line;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let line = decode_text(bytes);
    let _outcome = parse_sse_line(&line, "langgraph/agent", &UuidV4);
});
