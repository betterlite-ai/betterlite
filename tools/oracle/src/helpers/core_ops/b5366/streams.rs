use betterlite_core::message_utilities::response_streams::StreamError;
use betterlite_core::message_utilities::response_streams::iterators::{
    DeltaIdentity, ModelResponseIterator, ModelResponseListIterator,
};
use betterlite_core::message_utilities::response_streams::text_completion::TextCompletionStreamWrapper;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;

use super::errors;
use crate::helpers::HelperResult;

const DELTA_CHUNK_ID: &str = "b5366-delta";

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Call {
    TextCompletionStream(TextCompletionArgs),
    ModelResponseIterator(IteratorArgs),
    ModelResponseListIterator(ListIteratorArgs),
}

#[derive(Debug, Deserialize)]
pub(crate) struct TextCompletionArgs {
    chunks: Vec<Json>,
    #[serde(default)]
    include_usage: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct IteratorArgs {
    response: Json,
    #[serde(default)]
    convert_to_delta: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListIteratorArgs {
    responses: Vec<Json>,
    #[serde(default)]
    delay_seconds: Option<f64>,
}

pub(super) fn run(call: &Call) -> HelperResult {
    match call {
        Call::TextCompletionStream(args) => text_completion(args),
        Call::ModelResponseIterator(args) => iterator(args),
        Call::ModelResponseListIterator(args) => list_iterator(args),
    }
}

// LiteLLM wraps a text completion conversion failure in its provider exception, which carries a 500
fn text_completion(args: &TextCompletionArgs) -> HelperResult {
    let stream_options = args.include_usage.map(|flag| {
        let mut fields = IndexMap::new();
        fields.insert(String::from("include_usage"), Json::Bool(flag));
        Json::Object(fields)
    });
    let wrapper = TextCompletionStreamWrapper {
        chunks: args.chunks.iter().cloned(),
        stream_options,
    };
    wrapper
        .collect::<Result<Vec<Json>, StreamError>>()
        .map(|converted| json!(converted))
        .map_err(|error| errors::status_error("APIConnectionError", 500, error.to_string()))
}

fn iterator(args: &IteratorArgs) -> HelperResult {
    let identity = args.convert_to_delta.then_some(DeltaIdentity {
        id: DELTA_CHUNK_ID,
        created: 0,
    });
    let chunk = ModelResponseIterator::new(args.response.clone(), identity)
        .map_err(|error| errors::value_error(error.to_string()))?;
    Ok(json!(chunk.collect::<Vec<Json>>()))
}

fn list_iterator(args: &ListIteratorArgs) -> HelperResult {
    let iterator = ModelResponseListIterator::new(args.responses.clone(), args.delay_seconds)
        .map_err(|error| errors::value_error(error.to_string()))?;
    Ok(json!(iterator.collect::<Vec<Json>>()))
}
