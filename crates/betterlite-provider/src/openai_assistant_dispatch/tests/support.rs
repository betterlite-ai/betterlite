#![cfg(test)]

use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody, merge_query};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::assistants::client::ClientOptions;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::realtime::logging::{RealtimeLogRecord, RealtimeLogger};
use betterlite_openai::realtime::url::QueryParams;
use betterlite_openai::thread_runs::error::ThreadRunsError;
use betterlite_openai::thread_runs::stream::{RunChunks, RunStreamTransport};
use futures_util::stream;
use http::HeaderMap;
use secrecy::SecretString;

use crate::openai_assistant_dispatch::call::OpenAiAssistantOperation;
use crate::openai_assistant_dispatch::error::OpenAiAssistantError;
use crate::openai_assistant_dispatch::execute::dispatch;
use crate::openai_assistant_dispatch::{OpenAiAssistantRequest, OpenAiAssistantResult};
use crate::operation::{OperationRequest, OperationResult};

pub(super) const BASE: &str = "https://example.test/v1";
pub(super) const KEY: &str = "sk-fixture";
pub(super) const ERROR_BODY: &str = r#"{"error":{"message":"nope","type":"invalid_request_error","param":"model","code":"model_not_found"}}"#;

#[derive(Debug, Clone)]
pub(super) struct Seen {
    pub(super) method: String,
    pub(super) url: String,
    pub(super) authorization: Option<String>,
    pub(super) organization: Option<String>,
    pub(super) beta: Option<String>,
    pub(super) content_type: Option<String>,
    pub(super) body: Option<JsonValue>,
}

#[derive(Default)]
pub(super) struct Recorder {
    pub(super) seen: Mutex<Vec<Seen>>,
    replies: Mutex<VecDeque<(u16, String)>>,
    pub(super) stream: Mutex<Option<String>>,
}

impl Recorder {
    pub(super) fn queue(&self, status: u16, body: &str) {
        self.replies
            .lock()
            .unwrap()
            .push_back((status, body.to_owned()));
    }

    fn record(&self, request: &OutboundRequest) {
        let mut url = request.url.clone();
        merge_query(&mut url, &request.query);
        let body: Option<JsonValue> = match &request.body {
            RequestBody::Json(bytes) => Some(serde_json::from_slice(bytes).unwrap()),
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => None,
        };
        let mut content_type = header(&request.headers, "content-type");
        if content_type.is_none() && body.is_some() {
            content_type = Some(String::from("application/json"));
        }
        self.seen.lock().unwrap().push(Seen {
            method: request.method.as_str().to_owned(),
            url: url.as_str().to_owned(),
            authorization: header(&request.headers, "authorization"),
            organization: header(&request.headers, "openai-organization"),
            beta: header(&request.headers, "openai-beta"),
            content_type,
            body,
        });
    }
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    let value = headers.get(name)?;
    Some(value.to_str().unwrap().to_owned())
}

impl AssistantsTransport for Recorder {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.record(&request);
        let (status, body) = self.replies.lock().unwrap().pop_front().unwrap();
        ready(Ok(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body: body.into_bytes(),
        }))
    }
}

impl RunStreamTransport for Recorder {
    fn open(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<RunChunks, ThreadRunsError>> + Send {
        self.record(&request);
        let body = self.stream.lock().unwrap().take().unwrap_or_default();
        let chunks: RunChunks = Box::pin(stream::once(async move {
            Ok::<Vec<u8>, ThreadRunsError>(body.into_bytes())
        }));
        ready(Ok(chunks))
    }
}

#[derive(Debug)]
pub(super) struct RecordedCall {
    pub(super) api_base: String,
    pub(super) headers: Headers,
    pub(super) query_params: QueryParams,
}

#[derive(Default)]
pub(super) struct RecordingLogger {
    pub(super) calls: Vec<RecordedCall>,
}

impl RealtimeLogger for RecordingLogger {
    fn pre_call(&mut self, record: RealtimeLogRecord<'_>) -> Result<(), PrePostError> {
        self.calls.push(RecordedCall {
            api_base: record.api_base.to_owned(),
            headers: record.headers.clone(),
            query_params: record.query_params.clone(),
        });
        Ok(())
    }
}

pub(super) fn options() -> ClientOptions {
    ClientOptions {
        api_key: Some(SecretString::from(KEY)),
        api_base: Some(String::from(BASE)),
        timeout: None,
        max_retries: None,
        organization: Some(String::from("org-fixture")),
    }
}

pub(super) async fn run_operation(
    transport: &Arc<Recorder>,
    logger: &mut RecordingLogger,
    operation: OpenAiAssistantOperation,
) -> Result<OpenAiAssistantResult, OpenAiAssistantError> {
    let answer = dispatch(
        OperationRequest::OpenAiAssistant(OpenAiAssistantRequest),
        operation,
        Arc::clone(transport),
        logger,
    )
    .await?;
    let OperationResult::OpenAiAssistant(result) = answer else {
        panic!("the assistant composition returned another operation")
    };
    Ok(result)
}
