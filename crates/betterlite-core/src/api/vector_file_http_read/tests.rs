#![cfg(test)]

mod clients;
mod errors;
mod headers;
mod paths;
mod support;

use betterlite_models::decisions::Json;

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the expected JSON parses")
}

const RETRIEVE_BODY: &str = r#"{"id":"file_b","object":"vector_store.file","status":"completed"}"#;
const CONTENT_BODY: &str =
    r#"{"file_id":"file_b","filename":"b","content":[{"type":"text","text":"hello"}]}"#;
