#![no_main]

use std::collections::BTreeMap;

use betterlite_openai::ragflow_vector_store::decode::decode_ragflow_create;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_ragflow_create(200, &BTreeMap::new(), bytes);
});
