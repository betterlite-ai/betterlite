#![no_main]

use betterlite_anthropic::batches::decode::{BatchBody, decode_anthropic_batch_result};
use libfuzzer_sys::fuzz_target;

// the first byte selects the body: R decodes result rows, anything else a lifecycle object
fuzz_target!(|input: &[u8]| {
    let (operation, body) = match input.split_first() {
        Some((b'R', body)) => (BatchBody::Result, body),
        Some((_, body)) => (BatchBody::Lifecycle, body),
        None => return,
    };
    let _decoded = decode_anthropic_batch_result(operation, body);
});
