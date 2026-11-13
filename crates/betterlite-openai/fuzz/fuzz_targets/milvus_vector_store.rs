#![no_main]

use std::collections::BTreeMap;

use betterlite_openai::milvus_vector_store::decode::decode_milvus_search;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_milvus_search(200, &BTreeMap::new(), bytes, "query", "text");
});
