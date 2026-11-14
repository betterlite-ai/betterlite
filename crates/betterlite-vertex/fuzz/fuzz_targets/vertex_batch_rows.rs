#![no_main]

use betterlite_vertex::vertex_batch_rows::correlation::{
    CorrelationInput, decode_batch_correlation,
};
use betterlite_vertex::vertex_batch_rows::decode::{
    decode_vertex_batch_row, split_vertex_batch_key,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_batch_correlation(CorrelationInput::LabelChunks(&[
        bytes,
    ])));
    drop(decode_batch_correlation(CorrelationInput::Key(bytes)));
    if let Ok(row) = decode_vertex_batch_row(bytes) {
        drop(split_vertex_batch_key(&row));
    }
});
