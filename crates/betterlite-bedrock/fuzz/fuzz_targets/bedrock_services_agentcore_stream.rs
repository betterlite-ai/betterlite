#![no_main]

use betterlite_bedrock::agentcore_stream::line::decode_agentcore_line;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: (&[u8], u8)| {
    let size = usize::from(input.1).max(1);
    let mut buffer = Vec::new();
    let mut events = Vec::new();
    for chunk in input.0.chunks(size) {
        drop(decode_agentcore_line(&mut buffer, chunk, &mut events));
    }
});
