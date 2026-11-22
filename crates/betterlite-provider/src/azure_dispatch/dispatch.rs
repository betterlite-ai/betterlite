use std::collections::BTreeMap;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_prompts::arguments::json::{Json, Object};
use http::HeaderMap;
use serde::Serialize;

use crate::operation::{ChatCompletion, OperationError, OperationResult, success};
use crate::prepared::{PreparedHttp, PreparedKind, PreparedRequest, Upstream};

use self::reply::{
    chat_reply_view, chat_response, embedding_reply_view, embedding_response, text_response,
};
use super::AzureResult;
use super::call::{ChatCall, Composition, EmbeddingCall, TextCall};
use super::compose;
use super::error::AzureProviderError;

mod reply;

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

fn recorded(http: &PreparedHttp) -> Result<RecordedRequest, AzureProviderError> {
    let body = serde_json::from_slice(&http.body)
        .map_err(|source| AzureProviderError::Encode { source })?;
    let headers = http
        .headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                value.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    Ok(RecordedRequest {
        method: http.method.as_str().to_owned(),
        url: http.url.to_string(),
        headers,
        body,
    })
}

fn status_json(status: Option<u16>) -> Json {
    Json::Object(Object::from_iter([(
        String::from("status"),
        status.map_or(Json::Null, |code| Json::Number(code.into())),
    )]))
}

fn operation_status(error: &OperationError) -> Option<u16> {
    if let OperationError::Provider { response } = error {
        Some(response.status)
    } else {
        None
    }
}

// a Python failure that carries no status still leaves litellm.completion as an APIError with 500
fn failed(error: &AzureProviderError) -> AzureResult {
    AzureResult {
        requests: Vec::new(),
        reply: Json::Null,
        error: status_json(error.status().or(Some(500))),
    }
}

fn replied(record: RecordedRequest, result: Result<Json, OperationError>) -> AzureResult {
    match result {
        Ok(reply) => AzureResult {
            requests: vec![record],
            reply,
            error: Json::Null,
        },
        Err(error) => AzureResult {
            requests: vec![record],
            reply: Json::Null,
            error: status_json(operation_status(&error).or(Some(500))),
        },
    }
}

fn answer_upstream(http: &PreparedHttp, answer: &dyn Answer) -> UpstreamResponse {
    let reply = answer.answer(http);
    UpstreamResponse {
        status: reply.status,
        headers: reply.headers,
        body: reply.body,
    }
}

pub fn dispatch_chat(
    composition: &Composition,
    call: ChatCall,
    answer: &dyn Answer,
) -> AzureResult {
    let (http, json_mode) = match compose::prepare_chat_http(composition, call) {
        Ok(prepared) => prepared,
        Err(error) => return failed(&error),
    };
    let record = match recorded(&http) {
        Ok(record) => record,
        Err(error) => return failed(&error),
    };
    let upstream = answer_upstream(&http, answer);
    let reply = chat_response(upstream, json_mode).map(|response| chat_reply_view(&response));
    replied(record, reply)
}

pub fn dispatch_embedding(
    composition: &Composition,
    call: EmbeddingCall,
    answer: &dyn Answer,
) -> AzureResult {
    let http = match compose::prepare_embedding_http(composition, call) {
        Ok(http) => http,
        Err(error) => return failed(&error),
    };
    let record = match recorded(&http) {
        Ok(record) => record,
        Err(error) => return failed(&error),
    };
    let upstream = answer_upstream(&http, answer);
    let reply = embedding_response(upstream).map(|response| embedding_reply_view(&response));
    replied(record, reply)
}

// The bool from the composition is the text post_call condition; the typed dispatcher has no logger.
pub fn dispatch_text(
    composition: &Composition,
    call: TextCall,
    answer: &dyn Answer,
) -> AzureResult {
    let http = match compose::prepare_text_http(composition, call) {
        Ok((http, _post_call)) => http,
        Err(error) => return failed(&error),
    };
    let record = match recorded(&http) {
        Ok(record) => record,
        Err(error) => return failed(&error),
    };
    let upstream = answer_upstream(&http, answer);
    let reply = text_response(upstream).map(|response| chat_reply_view(&response));
    replied(record, reply)
}

// The three shapes the Azure request reaches; every other outcome is a fixture mistake, so each variant is named.
#[rustfmt::skip]
pub fn decode(
    prepared: &PreparedRequest,
    upstream: Upstream,
) -> Result<OperationResult, OperationError> {
    match (prepared.kind(), upstream) {
        (PreparedKind::Chat(_), Upstream::Buffered(response)) => chat_result(response),
        (PreparedKind::Stream(_), Upstream::Streaming(response)) => Ok(OperationResult::Stream(Box::new(response))),
        (PreparedKind::Stream(_), Upstream::Buffered(response)) => {
            success(response)?;
            Err(unsupported())
        }
        (PreparedKind::Embedding(_), Upstream::Buffered(response)) => embedding_result(response),
        (PreparedKind::Chat(_), Upstream::Streaming(_) | Upstream::Anthropic(_))
        | (PreparedKind::Stream(_), Upstream::Anthropic(_))
        | (PreparedKind::Embedding(_), Upstream::Streaming(_) | Upstream::Anthropic(_))
        | (PreparedKind::Other, Upstream::Buffered(_) | Upstream::Streaming(_) | Upstream::Anthropic(_)) => Err(unsupported()),
    }
}

fn unsupported() -> OperationError {
    OperationError::UnsupportedOperation {
        provider: String::from("azure"),
        operation: String::from("provider endpoint"),
    }
}

fn chat_result(response: UpstreamResponse) -> Result<OperationResult, OperationError> {
    Ok(OperationResult::Chat(ChatCompletion::OpenAi(Box::new(
        chat_response(response, false)?,
    ))))
}

fn embedding_result(response: UpstreamResponse) -> Result<OperationResult, OperationError> {
    Ok(OperationResult::Embedding(Box::new(embedding_response(
        response,
    )?)))
}
