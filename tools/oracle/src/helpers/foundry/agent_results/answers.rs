use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::PoisonError;

use async_trait::async_trait;
use betterlite_foundry::agent_requests::prepare::{Headers, prepare};
use betterlite_foundry::agent_requests::request::AgentCallRequest;
use betterlite_foundry::agent_results::error::AgentResultError;
use betterlite_foundry::agent_results::messages::{extract_content, transform_annotations};
use betterlite_foundry::agent_results::reply::{
    AgentReply, ReplyInput, TokenCountRequest, TokenCounter, build_model_response,
};
use betterlite_foundry::agent_results::request::build_stream_request;
use betterlite_foundry::agent_results::stream::{
    AgentChunk, AgentChunkStream, STATUS_OK, StreamIdentity, StreamSetup, open_stream,
};
use betterlite_http::error::HttpError;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_prompts::arguments::json::Object;
use betterlite_tokenizer::count::error::CountError;
use futures_util::StreamExt as _;
use futures_util::stream::{BoxStream, iter};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde_json::{Value, json};

use super::input::{AnnotationCall, ExtractionCall, ReplyCall, RequestCall, StreamCall};

pub(super) type Calls = Result<Value, Box<HelperError>>;

pub(super) fn extractions(calls: &[ExtractionCall]) -> Calls {
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        let extracted = extract_content(&call.messages);
        answers.push(json!({
            "content": extracted.content,
            "annotations": serde_json::to_value(extracted.annotations).or_config_error()?,
        }));
    }
    Ok(Value::Array(answers))
}

pub(super) fn annotations(calls: &[AnnotationCall]) -> Calls {
    let answers = calls
        .iter()
        .map(|call| transform_annotations(Some(&call.annotations)))
        .collect::<Vec<_>>();
    Ok(serde_json::to_value(answers).or_config_error()?)
}

pub(super) async fn replies(calls: &[ReplyCall]) -> Calls {
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        answers.push(reply_response(call).await?);
    }
    Ok(serde_json::to_value(answers).or_config_error()?)
}

async fn reply_response(call: &ReplyCall) -> Result<ReplyAnswer, Box<HelperError>> {
    let counter = RecordingCounter::new(call.fail);
    let input = ReplyInput {
        model: &call.model,
        content: &call.content,
        thread_id: &call.thread_id,
        messages: &call.messages,
        annotations: call.annotations.clone(),
    };
    let existing = AgentReply {
        model: None,
        choices: Vec::new(),
        hidden_params: call.hidden_params.clone(),
        usage: call.existing_usage,
    };
    let reply = build_model_response(existing, &input, &counter).await;
    Ok(ReplyAnswer {
        reply,
        counts: counter.records(),
    })
}

pub(super) async fn streams(calls: &[StreamCall]) -> Calls {
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        answers.push(stream_response(call).await?);
    }
    Ok(Value::Array(answers))
}

async fn stream_response(call: &StreamCall) -> Result<Value, Box<HelperError>> {
    let setup = StreamSetup {
        model: &call.model,
        status: STATUS_OK,
        identity: StreamIdentity {
            id: call.id.clone(),
            created: call.created,
        },
    };
    let stream = open_stream(setup, body(&call.body, call.split))
        .await
        .or_value_error()?;
    let chunks = collect_chunks(stream).await?;
    Ok(json!({
        "chunks": serde_json::to_value(chunks).or_config_error()?,
    }))
}

pub(super) async fn requests(calls: &[RequestCall]) -> Calls {
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        answers.push(answered_request(call).await?);
    }
    Ok(Value::Array(answers))
}

async fn answered_request(call: &RequestCall) -> Result<Value, Box<HelperError>> {
    let key = SecretString::from(call.api_key.clone());
    let incoming = secret_headers(&call.headers);
    let request = AgentCallRequest {
        model: &call.model,
        messages: &call.messages,
        optional_params: &call.optional_params,
        headers: &incoming,
        api_key: Some(&key),
        api_base: Some(&call.api_base),
    };
    let prepared = prepare(&request).or_value_error()?;
    let built = build_stream_request(
        &prepared,
        &call.messages,
        &call.optional_params,
        &call.litellm_params,
    )
    .or_value_error()?;
    let setup = StreamSetup {
        model: &call.model,
        status: call.status,
        identity: StreamIdentity {
            id: call.id.clone(),
            created: call.created,
        },
    };
    let outcome = outcome_of(setup, &call.body).await?;
    Ok(json!({
        "url": built.url,
        "headers": shown_headers(&built.headers),
        "body": serde_json::to_value(&built.payload).or_config_error()?,
        "ssl_verify": built.ssl_verify,
        "outcome": outcome,
    }))
}

async fn outcome_of(setup: StreamSetup<'_>, text: &str) -> Result<Value, Box<HelperError>> {
    match open_stream(setup, body(text, None)).await {
        Ok(stream) => {
            let chunks = collect_chunks(stream).await?;
            Ok(json!({
                "ok": true,
                "chunks": serde_json::to_value(chunks).or_config_error()?,
            }))
        }
        Err(error) => Ok(failed(&error)),
    }
}

fn failed(error: &AgentResultError) -> Value {
    json!({
        "ok": false,
        "status": error.status(),
        "message": error.to_string(),
    })
}

// a split inside a UTF-8 code point reaches the decoder as two chunks, the way a network read would
fn body(text: &str, split: Option<usize>) -> BoxStream<'static, Result<Vec<u8>, HttpError>> {
    let bytes = text.as_bytes();
    let Some(at) = split else {
        return iter([Ok::<Vec<u8>, HttpError>(bytes.to_vec())]).boxed();
    };
    let offset = at.min(bytes.len());
    iter([
        Ok::<Vec<u8>, HttpError>(bytes.get(..offset).unwrap_or_default().to_vec()),
        Ok::<Vec<u8>, HttpError>(bytes.get(offset..).unwrap_or_default().to_vec()),
    ])
    .boxed()
}

async fn collect_chunks(mut stream: AgentChunkStream) -> Result<Vec<AgentChunk>, Box<HelperError>> {
    let mut chunks = Vec::new();
    while let Some(item) = stream.next().await {
        chunks.push(item.or_value_error()?);
    }
    Ok(chunks)
}

fn shown_headers(headers: &Headers) -> BTreeMap<&str, &str> {
    headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect()
}

fn secret_headers(headers: &BTreeMap<String, String>) -> Headers {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
        .collect()
}

struct RecordingCounter {
    fail: bool,
    calls: Mutex<Vec<Recorded>>,
}

impl RecordingCounter {
    fn new(fail: bool) -> Self {
        Self {
            fail,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn records(&self) -> Vec<Recorded> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Serialize)]
struct ReplyAnswer {
    reply: AgentReply,
    counts: Vec<Recorded>,
}

#[derive(Debug, Clone, Serialize)]
struct Recorded {
    model: String,
    messages: Option<Vec<Object>>,
    text: Option<String>,
    count_response_tokens: bool,
}

#[async_trait]
impl TokenCounter for RecordingCounter {
    async fn count_request(&self, request: &TokenCountRequest<'_>) -> Result<u64, CountError> {
        let record = Recorded {
            model: request.model.to_owned(),
            messages: request.messages.map(<[Object]>::to_vec),
            text: request.text.map(str::to_owned),
            count_response_tokens: request.count_response_tokens,
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
        if self.fail {
            return Err(CountError::MissingInput);
        }
        if let Some(messages) = request.messages {
            let Ok(count) = u64::try_from(messages.len()) else {
                return Err(CountError::Overflow);
            };
            return Ok(count);
        }
        let Some(text) = request.text else {
            return Ok(0);
        };
        let Ok(count) = u64::try_from(text.chars().count()) else {
            return Err(CountError::Overflow);
        };
        Ok(count)
    }
}
