use betterlite_openai::batches::records::BatchObject;

use super::RoutingIdType;
use super::routing::encode_file_id_with_model;

pub fn encode_batch_response_ids(response: &mut BatchObject, model: &str) {
    if response.id.is_empty() {
        return;
    }
    response.id = encode_file_id_with_model(&response.id, model, RoutingIdType::Batch);
    for file_id in [&mut response.output_file_id, &mut response.error_file_id] {
        if let Some(value) = file_id.as_mut().filter(|value| !value.is_empty()) {
            *value = encode_file_id_with_model(value, model, RoutingIdType::File);
        }
    }
    if !response.input_file_id.is_empty() {
        response.input_file_id =
            encode_file_id_with_model(&response.input_file_id, model, RoutingIdType::File);
    }
}
