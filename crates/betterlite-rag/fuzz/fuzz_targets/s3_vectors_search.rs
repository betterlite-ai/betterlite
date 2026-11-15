#![no_main]

use std::collections::BTreeMap;

use betterlite_rag::s3_vectors_search::response::decode_s3_vectors_search;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_s3_vectors_search(200, &BTreeMap::new(), bytes, None);
});
