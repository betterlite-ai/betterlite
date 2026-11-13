#![no_main]

use betterlite_vertex::google_genai_guardrail::sse::decode_google_guardrail_sse;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_google_guardrail_sse(&chunk_list(bytes)));
});

// A stride taken from the front varies the chunk boundaries; the corpus replay splits a file the same way.
fn chunk_list(bytes: &[u8]) -> Vec<&[u8]> {
    let Some((first, rest)) = bytes.split_first() else {
        return Vec::new();
    };
    let stride = usize::from(*first).rem_euclid(7).saturating_add(1);
    rest.chunks(stride).collect()
}
