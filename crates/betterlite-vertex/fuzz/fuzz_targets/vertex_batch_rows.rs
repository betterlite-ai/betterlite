#![no_main]

use betterlite_vertex::vertex_batch_rows::correlation::{
    CorrelationInput, decode_batch_correlation,
};
use betterlite_vertex::vertex_batch_rows::decode::{
    decode_vertex_batch_row, is_embeddings_output_row, is_generate_output_row,
    split_vertex_batch_key,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let chunks: [&[u8]; 1] = [bytes];
    drop(decode_batch_correlation(CorrelationInput::LabelChunks(
        &chunks,
    )));
    drop(decode_batch_correlation(CorrelationInput::Key(bytes)));
    if let Ok(row) = decode_vertex_batch_row(bytes) {
        drop(split_vertex_batch_key(&row));
        drop(is_embeddings_output_row(&row));
        drop(is_generate_output_row(&row));
    }
});
