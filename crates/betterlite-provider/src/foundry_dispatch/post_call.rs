use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use indexmap::IndexMap;

use crate::openai_dispatch::logging::LoggedCall;
use crate::operation::OperationError;

// main.py's logging.post_call, as one port the composition writes and a fixture or the proxy reads back
pub trait FoundryLogger {
    fn post_call(&mut self, record: LoggedCall);
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoLogger;

impl FoundryLogger for NoLogger {
    fn post_call(&mut self, _record: LoggedCall) {}
}

#[must_use]
pub fn headers_json(headers: Option<&Headers>) -> Option<IndexMap<String, Json>> {
    headers.map(|map| {
        map.iter()
            .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
            .collect()
    })
}

// LiteLLM hands post_call the stream wrapper itself; this is the part of it a case can compare
#[must_use]
pub fn stream_report() -> Json {
    Json::Object(IndexMap::from([(String::from("stream"), Json::Bool(true))]))
}

#[must_use]
pub fn failure_report(error: &OperationError) -> Json {
    Json::Text(error.to_string())
}

// A buffered reply's model and first message text, the two fields both sides read off ModelResponse
#[must_use]
pub fn reply_report(model: Option<&str>, content: Option<&str>) -> Json {
    Json::Object(IndexMap::from([
        (
            String::from("model"),
            model.map_or(Json::Null, |text| Json::Text(text.to_owned())),
        ),
        (
            String::from("content"),
            content.map_or(Json::Null, |text| Json::Text(text.to_owned())),
        ),
    ]))
}
