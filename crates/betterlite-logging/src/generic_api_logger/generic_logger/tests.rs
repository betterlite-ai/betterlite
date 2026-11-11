#![cfg(test)]
use super::error::ApiError;
use super::retry::RetrySleeper;
use super::transport::{PostRequest, PostTransport};
use super::{ApiEventType, GenericApiLogger, LoggerDeps};
use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::generic_api_logger::generic_settings::{GenericApiArgs, GenericApiSettings, resolve};
use crate::generic_api_logger::tests::args;
use async_trait::async_trait;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;
const PAYLOAD: &str = r#"{"id":"chatcmpl-1","trace_id":"trace-1","session_id":"session-1","call_type":"completion","response_cost":0.5,"status":"success","status_fields":{"llm_api_status":"success","guardrail_status":"not_run"},"total_tokens":5,"prompt_tokens":2,"completion_tokens":3,"startTime":1704067200.0,"endTime":1704067201.5,"completionStartTime":1704067200.5,"response_time":1.0,"model_map_information":{},"model":"gpt-4o","api_base":"https://api.openai.com","metadata":{},"saved_cache_cost":0.0,"request_tags":["team-a"],"model_parameters":{},"hidden_params":{},"end_user":"user-1","cache_hit":false,"response":{"id":"chatcmpl-resp","usage":{"total_tokens":5}}}"#;
#[derive(Default)]
pub(super) struct RecordingTransport {
    pub(super) requests: Mutex<Vec<PostRequest>>,
    pub(super) replies: Mutex<VecDeque<Result<u16, ApiError>>>,
    blocked: AtomicBool,
    pub(super) gate: Notify,
    pub(super) started: Notify,
}
impl RecordingTransport {
    pub(super) fn reply(&self, outcome: Result<u16, ApiError>) {
        self.replies.lock().expect("the replies").push_back(outcome);
    }
    pub(super) fn block(&self) {
        self.blocked.store(true, Ordering::SeqCst);
    }
    pub(super) fn first(&self) -> PostRequest {
        self.requests.lock().expect("the requests")[0].clone()
    }
    pub(super) fn bodies(&self) -> Vec<Json> {
        let requests = self.requests.lock().expect("the requests");
        requests
            .iter()
            .map(|request| serde_json::from_str(&request.body).expect("a JSON body"))
            .collect()
    }
}
#[async_trait]
impl PostTransport for RecordingTransport {
    async fn post(&self, request: PostRequest) -> Result<u16, ApiError> {
        self.requests.lock().expect("the requests").push(request);
        if self.blocked.swap(false, Ordering::SeqCst) {
            self.started.notify_one();
            self.gate.notified().await;
        }
        self.replies
            .lock()
            .expect("the replies")
            .pop_front()
            .unwrap_or(Ok(200))
    }
}
#[derive(Default)]
pub(super) struct RecordingSleeper {
    pub(super) waits: Mutex<Vec<Duration>>,
}
#[async_trait]
impl RetrySleeper for RecordingSleeper {
    async fn sleep(&self, duration: Duration) {
        self.waits.lock().expect("the waits").push(duration);
    }
}
pub(super) fn ready(
    config: GenericApiSettings,
    use_v1: bool,
    batch_size: usize,
) -> (
    Arc<RecordingTransport>,
    Arc<RecordingSleeper>,
    GenericApiLogger,
) {
    let recorder = Arc::new(RecordingTransport::default());
    let sleeper = Arc::new(RecordingSleeper::default());
    let transport: Arc<dyn PostTransport> = Arc::<RecordingTransport>::clone(&recorder);
    let clock: Arc<dyn RetrySleeper> = Arc::<RecordingSleeper>::clone(&sleeper);
    let deps = LoggerDeps {
        transport,
        sleeper: clock,
        ids: || "generated-id".to_owned(),
    };
    let logger = GenericApiLogger::new(config, deps, use_v1, batch_size);
    (recorder, sleeper, logger)
}
pub(super) async fn deliver(logger: &GenericApiLogger, kind: ApiEventType) {
    match kind {
        ApiEventType::Success => logger.async_log_success_event(event()).await,
        ApiEventType::Failure => logger.async_log_failure_event(event()).await,
    }
    .expect("the event is accepted");
}
pub(super) fn entries(recorder: &RecordingTransport) -> Vec<Json> {
    let bodies = recorder.bodies();
    let Json::List(entries) = &bodies[0] else {
        panic!("a batch body is a list");
    };
    entries.clone()
}
pub(super) fn settings(arguments: GenericApiArgs) -> GenericApiSettings {
    resolve(arguments, &BTreeMap::new(), &IndexMap::new()).expect("settings resolve")
}
pub(super) fn payload() -> StandardLoggingPayload {
    serde_json::from_str(PAYLOAD).expect("a valid standard logging payload")
}
fn event() -> Arc<LoggerEvent> {
    Arc::new(LoggerEvent {
        payload: Arc::new(payload()),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    })
}
#[rstest]
#[case::success_listed(ApiEventType::Success, "llm_api_success", true)]
#[case::success_not_listed(ApiEventType::Success, "llm_api_failure", false)]
#[case::failure_listed(ApiEventType::Failure, "llm_api_failure", true)]
#[case::failure_not_listed(ApiEventType::Failure, "llm_api_success", false)]
#[tokio::test]
async fn callback_logger_sends_only_the_listed_event_types(
    #[case] kind: ApiEventType,
    #[case] listed: &str,
    #[case] sent: bool,
) {
    let mut arguments = args();
    arguments.event_types = Some(vec![listed.to_owned()]);
    let (recorder, _sleeper, logger) = ready(settings(arguments), false, 1);
    deliver(&logger, kind).await;
    logger.send_batch().await;
    assert_eq!(!recorder.bodies().is_empty(), sent);
}
#[tokio::test]
async fn callback_logger_sends_at_the_batch_size_with_the_standard_payload() {
    let (recorder, _sleeper, logger) = ready(settings(args()), false, 2);
    deliver(&logger, ApiEventType::Success).await;
    assert!(recorder.bodies().is_empty());
    deliver(&logger, ApiEventType::Success).await;
    let entries = entries(&recorder);
    assert_eq!(entries.len(), 2);
    let Json::Object(entry) = &entries[0] else {
        panic!("a standard entry is an object");
    };
    assert_eq!(entry.get("id"), Some(&Json::Text("chatcmpl-1".to_owned())));
}
#[tokio::test]
async fn callback_logger_uses_the_v1_payload_when_enabled() {
    let (recorder, _sleeper, logger) = ready(settings(args()), true, 10);
    deliver(&logger, ApiEventType::Success).await;
    logger.send_batch().await;
    let entries = entries(&recorder);
    let Json::Object(entry) = &entries[0] else {
        panic!("a v1 entry is an object");
    };
    assert_eq!(entry.len(), 13);
    assert_eq!(
        entry.get("id"),
        Some(&Json::Text("chatcmpl-resp".to_owned()))
    );
}
