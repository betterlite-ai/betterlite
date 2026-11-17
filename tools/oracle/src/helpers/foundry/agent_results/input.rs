use std::collections::BTreeMap;

use betterlite_foundry::agent_results::messages::MessagesData;
use betterlite_foundry::agent_results::reply::Usage;
use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) extractions: Option<Vec<ExtractionCall>>,
    pub(crate) annotations: Option<Vec<AnnotationCall>>,
    pub(crate) replies: Option<Vec<ReplyCall>>,
    pub(crate) streams: Option<Vec<StreamCall>>,
    pub(crate) requests: Option<Vec<RequestCall>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExtractionCall {
    pub(crate) messages: MessagesData,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AnnotationCall {
    pub(crate) annotations: Vec<Object>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyCall {
    pub(crate) model: String,
    pub(crate) content: String,
    pub(crate) thread_id: String,
    pub(crate) messages: Vec<Object>,
    pub(crate) annotations: Option<Vec<Object>>,
    pub(crate) existing_usage: Option<Usage>,
    pub(crate) hidden_params: Option<Object>,
    pub(crate) fail: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StreamCall {
    pub(crate) model: String,
    pub(crate) body: String,
    pub(crate) id: String,
    pub(crate) created: i64,
    // a byte offset that splits the body into two transport chunks, so a case can cut a UTF-8 code point
    #[serde(default)]
    pub(crate) split: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestCall {
    pub(crate) model: String,
    pub(crate) messages: Vec<Object>,
    pub(crate) optional_params: Object,
    pub(crate) litellm_params: Object,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) api_base: String,
    pub(crate) api_key: String,
    pub(crate) status: u16,
    pub(crate) body: String,
    pub(crate) id: String,
    pub(crate) created: i64,
}
