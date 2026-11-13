#![no_main]

use betterlite_vertex::vertex_batch_create::payload::{
    VertexEndpointResponse, decode_vertex_batch_payload,
};
use betterlite_vertex::vertex_batch_records::types::VertexBatchPredictionResponse;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_vertex_batch_payload::<VertexBatchPredictionResponse>(bytes));
    drop(decode_vertex_batch_payload::<VertexEndpointResponse>(bytes));
});
