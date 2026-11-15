#![no_main]

use betterlite_vertex::vertex_rerank::response::decode_vertex_rerank;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| drop(decode_vertex_rerank(bytes)));
