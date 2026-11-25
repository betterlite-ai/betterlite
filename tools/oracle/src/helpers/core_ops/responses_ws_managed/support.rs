use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use betterlite_core::api::responses_ws_history::error::FrameSinkError;
use betterlite_core::api::responses_ws_history::{FrameSink, ManagedResponsesWebSocket};
use betterlite_core::api::responses_ws_managed::run::{FrameReceiveError, FrameSource};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Map, Value, from_str, json};

#[derive(Clone, Deserialize)]
pub(super) struct ChunkRow {
    pub(super) kind: String,
    pub(super) value: Option<Json>,
    pub(super) event_type: Option<String>,
    pub(super) dump: Option<String>,
}

#[derive(Clone, Deserialize)]
pub(super) struct ScriptRow {
    pub(super) fail: Option<String>,
    pub(super) chunks: Vec<ChunkRow>,
    pub(super) raise_after: Option<usize>,
    pub(super) raise_message: String,
}

#[derive(Clone, Deserialize)]
pub(super) struct QuotaRow {
    pub(super) kind: String,
    pub(super) message: String,
}

#[derive(Deserialize)]
pub(super) struct SinkRow {
    pub(super) fails_at: Option<usize>,
    pub(super) message: String,
}

#[derive(Default)]
pub(super) struct Recorder {
    pub(super) frames: Vec<String>,
    pub(super) sends: usize,
    pub(super) calls: Vec<RecordedCall>,
    pub(super) quotas: Vec<RecordedQuota>,
    pub(super) trace: Vec<String>,
}

pub(super) struct RecordedCall {
    pub(super) model: String,
    pub(super) kwargs: IndexMap<String, Json>,
}

pub(super) struct RecordedQuota {
    pub(super) model: Option<String>,
    pub(super) input_tokens: i64,
    pub(super) output_tokens: i64,
}

pub(super) type Shared = Arc<Mutex<Recorder>>;

pub(super) fn lock<Value>(shared: &Mutex<Value>) -> MutexGuard<'_, Value> {
    match shared.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(super) struct Socket {
    shared: Shared,
    fails_at: Option<usize>,
    message: String,
}

#[async_trait]
impl FrameSink for Socket {
    async fn send_text(&self, frame: &str) -> Result<(), FrameSinkError> {
        let attempt = self.take_attempt();
        if Some(attempt) == self.fails_at {
            return Err(FrameSinkError {
                message: self.message.clone(),
            });
        }
        lock(&self.shared).frames.push(frame.to_owned());
        Ok(())
    }
}

impl Socket {
    fn take_attempt(&self) -> usize {
        let mut recorder = lock(&self.shared);
        recorder.sends = recorder.sends.saturating_add(1);
        recorder.sends
    }
}

pub(super) fn socket(shared: &Shared, row: &SinkRow) -> Socket {
    Socket {
        shared: Arc::clone(shared),
        fails_at: row.fails_at,
        message: row.message.clone(),
    }
}

pub(super) struct ScriptedSource {
    shared: Shared,
    frames: Vec<String>,
    disconnect: String,
    next: usize,
}

impl ScriptedSource {
    pub(super) fn new(shared: &Shared, frames: &[String], disconnect: &str) -> Self {
        Self {
            shared: Arc::clone(shared),
            frames: frames.to_vec(),
            disconnect: disconnect.to_owned(),
            next: 0,
        }
    }
}

#[async_trait]
impl FrameSource for ScriptedSource {
    async fn receive_text(&mut self) -> Result<String, FrameReceiveError> {
        lock(&self.shared).trace.push("receive".to_owned());
        match self.frames.get(self.next) {
            Some(frame) => {
                let received = frame.clone();
                self.next = self.next.saturating_add(1);
                Ok(received)
            }
            None => Err(FrameReceiveError {
                message: self.disconnect.clone(),
            }),
        }
    }
}

pub(super) fn frames(shared: &Shared) -> Result<Value, Box<HelperError>> {
    let recorded_frames = lock(shared).frames.clone();
    let mut parsed = Vec::with_capacity(recorded_frames.len());
    for frame in &recorded_frames {
        let value = from_str::<Value>(frame)
            .map_err(|error| helper_error("ValueError", error.to_string()))?;
        parsed.push(value);
    }
    Ok(Value::Array(parsed))
}

// the provider calls first, the quota calls second, each in the order the handler made them
pub(super) fn recorded(shared: &Shared) -> (Value, Value) {
    let recorder = lock(shared);
    let calls: Vec<Value> = recorder
        .calls
        .iter()
        .map(|call| json!({"model": call.model, "kwargs": call.kwargs}))
        .collect();
    let quotas: Vec<Value> = recorder
        .quotas
        .iter()
        .map(|quota| {
            json!({
                "model": quota.model,
                "input_tokens": quota.input_tokens,
                "output_tokens": quota.output_tokens,
            })
        })
        .collect();
    drop(recorder);
    (Value::Array(calls), Value::Array(quotas))
}

pub(super) fn history<Sink>(handler: &ManagedResponsesWebSocket<Sink>) -> Value
where
    Sink: FrameSink,
{
    let ordered: BTreeMap<&String, &Vec<IndexMap<String, Json>>> =
        handler.session_history.iter().collect();
    let mut stored = Map::new();
    for (response_id, messages) in ordered {
        stored.insert(response_id.clone(), json!(messages));
    }
    Value::Object(stored)
}

pub(super) fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}
