#![no_main]

use betterlite_guardrails::presidio_streams::coalesce::split_sse_preface;
use betterlite_guardrails::presidio_streams::unmask_sse::{PiiTokens, unmask_sse_bytes_chunk};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let tokens = PiiTokens::from([("<PERSON_1>".to_owned(), "Alice".to_owned())]);
    let _unmasked = unmask_sse_bytes_chunk(bytes, &tokens);
    let _split = split_sse_preface(bytes);
});
