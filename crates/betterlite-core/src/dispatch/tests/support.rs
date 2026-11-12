use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use betterlite_config::types::{LiteLlmParams, ModelDeployment, ModelInfo};
use betterlite_context::call_context::{BillingPolicy, CallContext, CallInputs};
use betterlite_http::error::HttpError;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_openai::chat::ChatRequest as ClientChatRequest;
use betterlite_openai::chat::deployment::{ChatDeployment, OpenAiDefaults};
use betterlite_openai::embedding::{
    EmbeddingRecord, EmbeddingRequest as ClientEmbeddingRequest, EmbeddingResponse, EmbeddingUsage,
    EmbeddingVector,
};
use futures_util::stream::{self, BoxStream, StreamExt as _};
use http::HeaderMap;
use secrecy::SecretString;
use serde_json::Number;
use uuid::Uuid;

use crate::dispatch::Entry;
use crate::dispatch::log::{CallLog, CallOutcome, CallRecord};
use crate::operation::chat::{
    ChatCall, ChatCallInput, ChatCompletionAnswer, ChatFailure, ChatRequest, ChatStreamAnswer,
};
use crate::operation::embedding::{
    EmbeddingCall, EmbeddingCallInput, EmbeddingFailure, EmbeddingRequest, EmbeddingResult,
};
use crate::rules::{RuleDecision, RuleEvaluator};

pub(crate) const MODEL_ID: &str = "dep-1";
const EMBEDDING: &[u8] = br#"{"model":"g","input":"hello"}"#;
const CHAT: &[u8] = br#"{"model":"g","messages":[{"role":"user","content":"hello"}]}"#;
const CHAT_STREAM: &[u8] =
    br#"{"model":"g","messages":[{"role":"user","content":"hello"}],"stream":true}"#;
const COMPLETION: &[u8] = br#"{"id":"chatcmpl-test","object":"chat.completion","created":1,"model":"up","choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}]}"#;

#[derive(Clone, Default)]
pub(crate) struct Trace {
    steps: Arc<Mutex<Vec<String>>>,
}

impl Trace {
    pub(crate) fn push(&self, step: &str) {
        self.steps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(step.to_owned());
    }

    pub(crate) fn steps(&self) -> Vec<String> {
        self.steps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RecordedCall {
    pub(crate) entry: Entry,
    pub(crate) call_id: String,
    pub(crate) outcome: CallOutcome,
    pub(crate) billing: BillingPolicy,
    pub(crate) privacy: bool,
    pub(crate) message: Option<String>,
}

pub(crate) struct RecordingLog {
    pub(crate) trace: Trace,
    pub(crate) calls: Arc<Mutex<Vec<RecordedCall>>>,
}

impl RecordingLog {
    pub(crate) fn new(trace: Trace) -> Self {
        Self {
            trace,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn calls(&self) -> Vec<RecordedCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CallLog for RecordingLog {
    fn record(&self, record: CallRecord<'_>) {
        let step = match record.outcome {
            CallOutcome::Succeeded => "success",
            CallOutcome::Failed => "failure",
            CallOutcome::Cancelled => "cancelled",
        };
        self.trace.push(step);
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedCall {
                entry: record.entry,
                call_id: record.call_id.to_owned(),
                outcome: record.outcome,
                billing: record.billing,
                privacy: record.privacy,
                message: record
                    .message
                    .map(str::to_owned)
                    .or_else(|| record.error.map(ToString::to_string)),
            });
    }
}

pub(crate) struct RecordingRules {
    pub(crate) trace: Trace,
    pub(crate) inputs: Arc<Mutex<Vec<String>>>,
    pub(crate) decision: Option<RuleDecision>,
}

impl RecordingRules {
    pub(crate) fn new(trace: Trace, decision: Option<RuleDecision>) -> Self {
        Self {
            trace,
            inputs: Arc::new(Mutex::new(Vec::new())),
            decision,
        }
    }

    pub(crate) fn inputs(&self) -> Vec<String> {
        self.inputs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RuleEvaluator for RecordingRules {
    fn evaluate(&self, index: usize, input: &str) -> Option<RuleDecision> {
        self.trace.push("rules");
        self.inputs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(input.to_owned());
        if index == 0 {
            self.decision.clone()
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamEnd {
    Ends,
    Pending,
}

#[derive(Clone, Debug)]
pub(crate) struct SeenChat {
    pub(crate) model_id: String,
    pub(crate) timeout: Duration,
    pub(crate) stream: bool,
}

pub(crate) struct FakeChat {
    pub(crate) trace: Trace,
    pub(crate) chunk_size: Option<u64>,
    pub(crate) stream_end: StreamEnd,
    pub(crate) seen: Arc<Mutex<Vec<SeenChat>>>,
}

impl FakeChat {
    pub(crate) fn new(trace: Trace, stream_end: StreamEnd) -> Self {
        Self {
            trace,
            chunk_size: None,
            stream_end,
            seen: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn seen(&self) -> Vec<SeenChat> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn begin(&self, step: &str, call: &ChatCallInput<'_>) {
        self.trace.push(step);
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SeenChat {
                model_id: call.deployment.model_id.clone(),
                timeout: call.deployment.timeout,
                stream: call.request.stream,
            });
    }
}

#[async_trait]
impl ChatCall for FakeChat {
    fn prepare(&self, _request: &ClientChatRequest) -> Result<ControlOptions, ChatFailure> {
        self.trace.push("prepare");
        ControlOptions::new(self.chunk_size).map_err(ChatFailure::from)
    }

    async fn complete(
        &self,
        call: ChatCallInput<'_>,
        _control: ControlOptions,
    ) -> Result<ChatCompletionAnswer, ChatFailure> {
        self.begin("complete", &call);
        Ok(ChatCompletionAnswer {
            body: COMPLETION.to_vec(),
            upstream_headers: HeaderMap::new(),
            upstream_time: Duration::ZERO,
        })
    }

    async fn stream(
        &self,
        call: ChatCallInput<'_>,
        _control: ControlOptions,
    ) -> Result<ChatStreamAnswer, ChatFailure> {
        self.begin("stream", &call);
        let body = body_stream(self.stream_end);
        let (upstream_headers, upstream_time) = (HeaderMap::new(), Duration::ZERO);
        Ok(ChatStreamAnswer {
            body,
            upstream_headers,
            upstream_time,
        })
    }
}

pub(crate) struct FakeEmbedding {
    pub(crate) trace: Trace,
    pub(crate) seen: Arc<Mutex<Vec<Duration>>>,
}

impl FakeEmbedding {
    pub(crate) fn new(trace: Trace) -> Self {
        Self {
            trace,
            seen: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn seen(&self) -> Vec<Duration> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl EmbeddingCall for FakeEmbedding {
    async fn embed(
        &self,
        call: EmbeddingCallInput<'_>,
    ) -> Result<EmbeddingResult, EmbeddingFailure> {
        self.trace.push("embed");
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.deployment.timeout);
        Ok(EmbeddingResult {
            response: embedding_response(),
            upstream_headers: HeaderMap::new(),
            upstream_time: Duration::ZERO,
        })
    }
}

pub(crate) fn deployment(model_id: &str, timeout: Duration) -> ChatDeployment {
    let listed = ModelDeployment {
        model_name: String::from("g"),
        litellm_params: LiteLlmParams {
            ssl_verify: None,
            model: Some(String::from("openai/up")),
            api_base: Some(String::from("http://127.0.0.1:9/v1")),
            api_key: Some(SecretString::from("configured-key")),
            api_version: None,
            timeout: None,
            stream_timeout: None,
            max_retries: None,
            rpm: None,
            tpm: None,
            weight: None,
        },
        model_info: ModelInfo::default(),
    };
    let mut deployment =
        ChatDeployment::resolve(&listed, &OpenAiDefaults::default()).expect("deployment");
    model_id.clone_into(&mut deployment.model_id);
    deployment.timeout = timeout;
    deployment
}

pub(crate) fn context(deadline: Option<SystemTime>, start: SystemTime) -> CallContext<'static> {
    CallContext::start(CallInputs {
        now: move || start,
        id: Uuid::nil,
        deadline,
        privacy: false,
    })
}

pub(crate) fn chat_request<Calls>(
    call_id: &str,
    model_id: &str,
    calls: Arc<Calls>,
    stream: bool,
) -> ChatRequest
where
    Calls: ChatCall + 'static,
{
    let body = [CHAT, CHAT_STREAM][usize::from(stream)];
    let request = Box::new(ClientChatRequest::parse(body).expect("chat request"));
    ChatRequest {
        request,
        calls,
        call_id: call_id.to_owned(),
        model_id: model_id.to_owned(),
    }
}

pub(crate) fn embedding_request(
    call_id: &str,
    model_id: &str,
    calls: Arc<dyn EmbeddingCall>,
) -> EmbeddingRequest {
    let request = ClientEmbeddingRequest::parse(EMBEDDING).expect("embedding request");
    EmbeddingRequest {
        call_id: call_id.to_owned(),
        model_id: model_id.to_owned(),
        request: Box::new(request),
        calls,
    }
}

fn embedding_response() -> EmbeddingResponse {
    EmbeddingResponse {
        object: String::from("list"),
        data: vec![EmbeddingRecord {
            object: String::from("embedding"),
            embedding: EmbeddingVector::Floats(vec![Number::from_f64(0.5).expect("finite number")]),
            index: 0,
        }],
        model: String::from("up"),
        usage: EmbeddingUsage {
            prompt_tokens: 2,
            completion_tokens: 0,
            completion_tokens_details: None,
            prompt_tokens_details: None,
            total_tokens: 2,
        },
    }
}

fn body_stream(end: StreamEnd) -> BoxStream<'static, Result<Vec<u8>, HttpError>> {
    match end {
        StreamEnd::Ends => {
            stream::iter(vec![Ok(b"chunk-one".to_vec()), Ok(b"chunk-two".to_vec())]).boxed()
        }
        StreamEnd::Pending => stream::iter(vec![Ok(b"chunk-one".to_vec())])
            .chain(stream::pending::<Result<Vec<u8>, HttpError>>())
            .boxed(),
    }
}
