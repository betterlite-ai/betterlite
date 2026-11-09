#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::ApiError;
use super::retry::RetrySleeper;
use super::transport::{PostRequest, PostTransport};
use super::{GenericApiLogger, LoggerDeps};
use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::generic_api_logger::generic_settings::{GenericApiArgs, GenericApiSettings, resolve};
use crate::generic_api_logger::tests::args;

const PAYLOAD: &str = r#"{"id":"chatcmpl-1","trace_id":"trace-1","session_id":"session-1","call_type":"completion","response_cost":0.5,"status":"success","status_fields":{"llm_api_status":"success","guardrail_status":"not_run"},"total_tokens":5,"prompt_tokens":2,"completion_tokens":3,"startTime":1704067200.0,"endTime":1704067201.5,"completionStartTime":1704067200.5,"response_time":1.0,"model_map_information":{},"model":"gpt-4o","api_base":"https://api.openai.com","metadata":{},"saved_cache_cost":0.0,"request_tags":["team-a"],"model_parameters":{},"hidden_params":{},"end_user":"user-1","cache_hit":false,"response":{"id":"chatcmpl-resp","usage":{"total_tokens":5}}}"#;

#[derive(Default)]
pub(super) struct RecordingTransport {
    pub(super) requests: Mutex<Vec<PostRequest>>,
    pub(super) replies: Mutex<VecDeque<Result<u16, ApiError>>>,
}

impl RecordingTransport {
    pub(super) fn reply(&self, outcome: Result<u16, ApiError>) {
        self.replies.lock().expect("the replies").push_back(outcome);
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

pub(super) struct Fixture {
    pub(super) recorder: Arc<RecordingTransport>,
    pub(super) sleeper: Arc<RecordingSleeper>,
    pub(super) logger: GenericApiLogger,
}

impl Fixture {
    pub(super) fn new(config: GenericApiSettings, use_v1: bool, batch_size: usize) -> Self {
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
        Self {
            recorder,
            sleeper,
            logger,
        }
    }

    pub(super) async fn success(&self) {
        self.logger
            .async_log_success_event(event())
            .await
            .expect("the success event is accepted");
    }

    pub(super) async fn failure(&self) {
        self.logger
            .async_log_failure_event(event())
            .await
            .expect("the failure event is accepted");
    }

    pub(super) async fn send(&self) {
        self.logger.send_batch().await;
    }

    pub(super) fn entries(&self) -> Vec<Json> {
        let bodies = self.recorder.bodies();
        let Json::List(entries) = &bodies[0] else {
            panic!("a batch body is a list");
        };
        entries.clone()
    }
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
        stream: false,
        guardrail_snapshot: None,
    })
}

#[tokio::test]
async fn callback_logger_queues_the_standard_payload_on_success() {
    let fixture = Fixture::new(settings(args()), false, 10);
    fixture.success().await;
    fixture.send().await;
    let entries = fixture.entries();
    let Json::Object(entry) = &entries[0] else {
        panic!("a standard entry is an object");
    };
    assert_eq!(entry.get("id"), Some(&Json::Text("chatcmpl-1".to_owned())));
}

#[tokio::test]
async fn callback_logger_queues_the_standard_payload_on_failure() {
    let fixture = Fixture::new(settings(args()), false, 10);
    fixture.failure().await;
    fixture.send().await;
    assert_eq!(fixture.entries().len(), 1);
}
#[tokio::test]
async fn callback_logger_skips_an_event_type_that_is_not_listed() {
    let mut arguments = args();
    arguments.event_types = Some(vec!["llm_api_success".to_owned()]);
    let fixture = Fixture::new(settings(arguments), false, 1);
    fixture.failure().await;
    fixture.send().await;
    assert!(fixture.recorder.bodies().is_empty());
}

#[tokio::test]
async fn callback_logger_sends_when_the_batch_size_is_reached() {
    let fixture = Fixture::new(settings(args()), false, 2);
    fixture.success().await;
    assert!(fixture.recorder.bodies().is_empty());
    fixture.success().await;
    assert_eq!(fixture.entries().len(), 2);
}

#[tokio::test]
async fn callback_logger_uses_the_v1_payload_when_enabled() {
    let fixture = Fixture::new(settings(args()), true, 10);
    fixture.success().await;
    fixture.send().await;
    let entries = fixture.entries();
    let Json::Object(entry) = &entries[0] else {
        panic!("a v1 entry is an object");
    };
    assert_eq!(entry.len(), 13);
    assert_eq!(
        entry.get("id"),
        Some(&Json::Text("chatcmpl-resp".to_owned()))
    );
}
