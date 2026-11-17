use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::vector_stores::fields::{attach_to_choices, from_request_data};

const DELTA: &str = "delta";
const NO_FIELDS: &str = "No search results or search failures found for streaming chunk";
const ERROR_PREFIX: &str = "Error adding search results to streaming chunk: ";

pub fn decorate_chunk(request_data: &IndexMap<String, Json>, chunk: &mut Json) -> Vec<String> {
    let fields = match from_request_data(request_data) {
        Ok(fields) => fields,
        Err(error) => return vec![format!("{ERROR_PREFIX}{error}")],
    };
    if fields.search_results.is_empty() && fields.failures.is_empty() {
        return vec![NO_FIELDS.to_owned()];
    }
    let Json::Object(record) = chunk else {
        return Vec::new();
    };
    if let Err(error) = attach_to_choices(record, DELTA, fields) {
        return vec![format!("{ERROR_PREFIX}{error}")];
    }
    Vec::new()
}
