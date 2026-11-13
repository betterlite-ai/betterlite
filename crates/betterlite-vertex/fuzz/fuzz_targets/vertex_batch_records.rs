#![no_main]

use betterlite_models::decisions::Json;
use betterlite_vertex::gemini_usage::usage::calculate_usage;
use betterlite_vertex::vertex_batch_records::model::decode_vertex_batch_identifier;
use betterlite_vertex::vertex_batch_records::rows::native_vertex_row_usage;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_vertex_batch_identifier(bytes));
    if let Ok(Json::Object(fields)) = serde_json::from_slice::<Json>(bytes)
        && let Some(Json::Object(response)) = fields.get("response")
    {
        drop(native_vertex_row_usage(response, &calculate_usage));
    }
});
