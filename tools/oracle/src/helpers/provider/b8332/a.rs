use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{StreamResponse, UpstreamResponse};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_openai::huggingface_chat::mapping::HubResponse;
use betterlite_openai::huggingface_embedding::response::OrdinaryEncoder;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{Hub, builtin_entries};
use betterlite_provider::api::rerank_contracts::{BaseRerankConfig, EnvironmentRequest};
use betterlite_provider::model_hub_dispatch::credentials::Globals;
use betterlite_provider::model_hub_dispatch::embedding::{EmbeddingCall, embed};
use betterlite_provider::model_hub_dispatch::hub::HubGetAsync;
use betterlite_provider::model_hub_dispatch::predibase;
use betterlite_provider::model_hub_dispatch::replicate;
use betterlite_provider::model_hub_dispatch::rerank::huggingface_rerank_config;
use betterlite_provider::model_hub_dispatch::transport::PostTransport;
use betterlite_provider::model_hub_dispatch::{
    CallLogging, ChatContext, DispatchDeps, HubDispatchError, ReplicateServices, huggingface,
};
use betterlite_provider::oci_common::Clock;
use betterlite_provider::operation::OperationError;
use betterlite_provider::predibase::reply::TokenCounter as PredibaseTokenCounter;
use betterlite_provider::prepared::PreparedRequest;
use betterlite_provider::replicate::poll::ReplicateSleeper;
use betterlite_provider::replicate::poll::transport::{
    ReplicateRequest, ReplicateResponse, ReplicateTransport,
};
use betterlite_provider::replicate::reply::{
    PostCall, PostCallLogger, TokenCounter as ReplicateTokenCounter,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_tokenizer::count::error::CountError;
use chrono::{DateTime, Utc};
use http::HeaderMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    HuggingfaceChat,
    HuggingfaceEmbedding,
    HuggingfaceRerank,
    Replicate,
    Predibase,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Case {
    op: Operation,
    model: String,
    #[serde(default)]
    messages: Vec<Message>,
    #[serde(default)]
    optional_params: Object,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    input_type: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    tenant_id: Option<String>,
    #[serde(default)]
    input: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    replies: Vec<Reply>,
    #[serde(default)]
    polls: Vec<Reply>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    #[serde(default = "default_status")]
    status: u16,
    body: String,
}

const fn default_status() -> u16 {
    200
}

struct Discard;

impl LogSink for Discard {
    fn emit(&self, _record: LogRecord) {}
}

fn helper_failure<Message: Display>(class: &str, message: Message) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

struct NoHub;

#[async_trait]
impl HubGetAsync for NoHub {
    async fn get(
        &self,
        _url: &str,
        _headers: &Headers,
        _query: &[(&str, &str)],
    ) -> Result<HubResponse, HubDispatchError> {
        Err(HubDispatchError::BodyNotObject)
    }
}

struct ScriptedPost {
    replies: Mutex<Vec<UpstreamResponse>>,
    requests: Mutex<Vec<(String, Headers, String)>>,
}

#[async_trait]
impl PostTransport for ScriptedPost {
    async fn post(
        &self,
        url: &str,
        headers: &Headers,
        body: &[u8],
        _deadline: Duration,
    ) -> Result<UpstreamResponse, HubDispatchError> {
        let sent = String::from_utf8_lossy(body).into_owned();
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((url.to_owned(), headers.clone(), sent));
        let mut replies = self.replies.lock().unwrap_or_else(PoisonError::into_inner);
        if replies.is_empty() {
            return Err(HubDispatchError::BodyNotObject);
        }
        Ok(replies.remove(0))
    }

    async fn post_stream(
        &self,
        _url: &str,
        _headers: &Headers,
        _body: &[u8],
        _deadline: Duration,
    ) -> Result<StreamResponse, HubDispatchError> {
        Err(HubDispatchError::Operation(
            OperationError::UnsupportedOperation {
                provider: String::from("b8332"),
                operation: String::from("stream"),
            },
        ))
    }
}

struct ScriptedTransport {
    replies: Mutex<Vec<ReplicateResponse>>,
    requests: Mutex<Vec<ReplicateRequest>>,
}

#[async_trait]
impl ReplicateTransport for ScriptedTransport {
    async fn send(&self, request: ReplicateRequest) -> Result<ReplicateResponse, TransportError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let mut replies = self.replies.lock().unwrap_or_else(PoisonError::into_inner);
        if replies.is_empty() {
            return Err(TransportError::Closed);
        }
        Ok(replies.remove(0))
    }
}

struct Sleeper;

#[async_trait]
impl ReplicateSleeper for Sleeper {
    async fn sleep(&self, _seconds: f64) {}
}

struct Logger;

impl PostCallLogger for Logger {
    fn post_call(&self, _call: PostCall<'_>) {}
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).unwrap_or_default()
    }
}

struct LengthEncoder;

impl OrdinaryEncoder for LengthEncoder {
    fn encode_ordinary(&self, text: &str) -> usize {
        text.chars().count()
    }
}

struct ReplicateCounting;

#[async_trait]
impl ReplicateTokenCounter for ReplicateCounting {
    async fn count_messages(&self, _model: &str, messages: &[Message]) -> Result<i64, CountError> {
        i64::try_from(messages.len()).map_err(|_overflow| CountError::Overflow)
    }

    async fn count_response_tokens(&self, _model: &str, text: &str) -> Result<i64, CountError> {
        i64::try_from(text.chars().count()).map_err(|_overflow| CountError::Overflow)
    }
}

struct PlainCounting;

#[async_trait]
impl PredibaseTokenCounter for PlainCounting {
    async fn count_messages(&self, messages: &[Message]) -> Option<i64> {
        i64::try_from(messages.len()).ok()
    }

    async fn count_text(&self, text: &str) -> Option<i64> {
        i64::try_from(text.chars().count()).ok()
    }
}

