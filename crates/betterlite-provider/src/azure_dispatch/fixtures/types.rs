use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Json;
use http::HeaderMap;
use serde::Serialize;

use crate::prepared::PreparedHttp;

#[derive(Debug, Clone)]
pub struct AnswerReply {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

// The answer port: the typed dispatcher sends the request it composed and never opens a connection itself
pub trait Answer {
    fn answer(&self, request: &PreparedHttp) -> AnswerReply;
}

// The request the dispatch composed, in the shape an oracle case compares.
#[derive(Debug, Clone, Serialize)]
pub struct RecordedRequest {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: Json,
}

// The typed dispatcher's answer: the requests it composed, the reply it projected, and the failure it met
#[derive(Debug, Clone, Serialize)]
pub struct AzureResult {
    pub requests: Vec<RecordedRequest>,
    pub reply: Json,
    pub error: Json,
}
