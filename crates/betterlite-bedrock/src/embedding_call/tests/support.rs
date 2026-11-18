use betterlite_models::decisions::Json;
use betterlite_openai::cohere_legacy_embedding::decode::LegacyEmbeddingResponse;
use betterlite_openai::embedding::EmbeddingVector;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use indexmap::IndexMap;

use super::fixture::Run;
use crate::bedrock_embedding_io::response::EmbeddingReply;
use crate::embedding_call::EmbeddingCallResponse;

pub(super) fn batch(response: &EmbeddingCallResponse) -> &EmbeddingReply {
    match response {
        EmbeddingCallResponse::Batch(reply) => reply,
        EmbeddingCallResponse::Legacy(_) => panic!("the call returns a batch reply"),
    }
}

pub(super) fn legacy(response: &EmbeddingCallResponse) -> &LegacyEmbeddingResponse {
    match response {
        EmbeddingCallResponse::Legacy(reply) => reply,
        EmbeddingCallResponse::Batch(_) => panic!("the call returns a legacy reply"),
    }
}

pub(super) fn vector(embedding: &EmbeddingVector) -> Vec<f64> {
    match embedding {
        EmbeddingVector::Floats(numbers) => numbers
            .iter()
            .map(|number| number.as_f64().expect("the vector holds finite numbers"))
            .collect(),
        EmbeddingVector::Base64(_) => panic!("the fixture expects a float vector"),
    }
}

pub(super) fn fields(value: &Json) -> &IndexMap<String, Json> {
    match value {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the logged value is an object")
        }
    }
}

pub(super) fn argument_fields(snapshot: &Json) -> &IndexMap<String, Json> {
    let Some(Json::Object(arguments)) = fields(snapshot).get("additional_args") else {
        panic!("the snapshot holds its additional args");
    };
    arguments
}

pub(super) fn phases(snapshots: &[Json]) -> Vec<&str> {
    snapshots
        .iter()
        .map(|snapshot| match fields(snapshot).get("log_event_type") {
            Some(Json::Text(phase)) => phase.as_str(),
            _ => panic!("the snapshot names its phase"),
        })
        .collect()
}

// post_call stores an object reply as its Python-style JSON text, so the fixture parses it back
pub(super) fn logged_response(run: &Run) -> Json {
    match run.original_response() {
        Some(Json::Text(text)) => serde_json::from_str(text).expect("the logged reply parses"),
        Some(other) => other.clone(),
        None => panic!("the call logs its reply"),
    }
}

pub(super) fn titan_body(embedding: &str, tokens: u32) -> String {
    format!("{{\"embedding\":{embedding},\"inputTextTokenCount\":{tokens}}}")
}

pub(super) fn decisions_object(value: &Object) -> Json {
    serde_json::from_value(serde_json::to_value(value).expect("the body serializes"))
        .expect("the body converts")
}

pub(super) fn decisions_value(value: &PromptJson) -> Json {
    serde_json::from_value(serde_json::to_value(value).expect("the reply serializes"))
        .expect("the reply converts")
}
