use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use crate::operation::OperationError;
use crate::prepared::PreparedHttp;

use super::super::call::{Composition, Request};
use super::super::compose::{self, Prepared, ReplyMode};
use super::super::error::AzureProviderError;
use super::super::reply::{Projection, embedding_response, project, reply_provider};
use super::super::resolve::convert;
use super::types::{Answer, AnswerReply, AzureResult, RecordedRequest};

pub(crate) fn recorded(http: &PreparedHttp) -> Result<RecordedRequest, AzureProviderError> {
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
        match status {
            Some(code) => Json::Number(Number::from(code)),
            None => Json::Null,
        },
    )]))
}

fn operation_status(error: &OperationError) -> Option<u16> {
    if let OperationError::Provider { response } = error {
        return Some(response.status);
    }
    None
}

// a Python failure that carries no status still leaves litellm.completion as an APIError with 500
fn failed(error: &AzureProviderError) -> AzureResult {
    AzureResult {
        requests: Vec::new(),
        reply: Json::Null,
        error: status_json(Some(error.status().unwrap_or(500))),
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
            error: status_json(Some(operation_status(&error).unwrap_or(500))),
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

// The two reply shapes the typed dispatcher projects, so one view reads both.
enum Reply {
    Chat(ChatResponse),
    Embedding(EmbeddingResponse),
}

fn view(reply: &Reply) -> Json {
    match reply {
        Reply::Chat(response) => {
            let first = response.choices.first();
            let content = first.and_then(|choice| choice.message.content.clone());
            let finish_reason = first.and_then(|choice| choice.finish_reason.clone());
            Json::Object(Object::from_iter([
                (String::from("id"), Json::Text(response.id.clone())),
                (String::from("object"), Json::Text(response.object.clone())),
                (String::from("model"), Json::Text(response.model.clone())),
                (
                    String::from("content"),
                    content.map_or(Json::Null, Json::Text),
                ),
                (
                    String::from("finish_reason"),
                    finish_reason.map_or(Json::Null, Json::Text),
                ),
                (
                    String::from("total_tokens"),
                    Json::Number(Number::from(response.usage.total_tokens)),
                ),
            ]))
        }
        Reply::Embedding(response) => {
            let embedding = response.data.first().map_or(Json::Null, |record| {
                convert(&record.embedding).unwrap_or(Json::Null)
            });
            Json::Object(Object::from_iter([
                (String::from("object"), Json::Text(response.object.clone())),
                (String::from("model"), Json::Text(response.model.clone())),
                (String::from("embedding"), embedding),
                (
                    String::from("total_tokens"),
                    Json::Number(Number::from(response.usage.total_tokens)),
                ),
            ]))
        }
    }
}

// The production projection for the branch the composition selected.
fn projected(prepared: &Prepared, upstream: UpstreamResponse) -> Result<Reply, OperationError> {
    match prepared.mode {
        ReplyMode::Chat { json_mode } => project(
            upstream,
            Projection::Chat {
                json_mode,
                provider: reply_provider(prepared.detection.as_deref()),
            },
        )
        .map(Reply::Chat),
        ReplyMode::Text { .. } => project(upstream, Projection::Text).map(Reply::Chat),
        ReplyMode::Embedding => embedding_response(upstream).map(Reply::Embedding),
    }
}

// The one dispatch entry point: compose, ask the port, project the answer through the production reply code.
pub fn dispatch(composition: &Composition, request: Request, answer: &dyn Answer) -> AzureResult {
    let prepared = match compose::prepare(composition, request) {
        Ok(prepared) => prepared,
        Err(error) => return failed(&error),
    };
    let record = match recorded(&prepared.http) {
        Ok(record) => record,
        Err(error) => return failed(&error),
    };
    let upstream = answer_upstream(&prepared.http, answer);
    let reply = projected(&prepared, upstream).map(|reply| view(&reply));
    replied(record, reply)
}
