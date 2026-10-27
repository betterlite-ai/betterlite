use std::sync::{Arc, Mutex};

use serde_json::Value;

use super::model::{Kwargs, MlflowEvent, Span, SpanKind, Status};

#[derive(Debug, thiserror::Error)]
pub enum MlflowError {
    #[error("MLflow operation failed: {0}")]
    Operation(String),
}

pub trait MlflowClient: Send + Sync {
    fn active_span(&self) -> Result<Option<Span>, MlflowError>;
    fn start_trace(&self, request: StartRequest) -> Result<Span, MlflowError>;
    fn start_span(
        &self,
        request: StartRequest,
        trace_id: &str,
        parent_id: &str,
    ) -> Result<Span, MlflowError>;
    fn add_event(&self, span: &Span, event: MlflowEvent) -> Result<(), MlflowError>;
    fn end_trace(
        &self,
        span: &Span,
        outputs: &Value,
        status: Status,
        end_time_ns: i64,
    ) -> Result<(), MlflowError>;
    fn end_span(
        &self,
        span: &Span,
        outputs: &Value,
        status: Status,
        end_time_ns: i64,
    ) -> Result<(), MlflowError>;
}

#[derive(Clone, Debug)]
pub struct StartRequest {
    pub name: String,
    pub kind: SpanKind,
    pub inputs: Kwargs,
    pub attributes: Kwargs,
    pub tags: Kwargs,
    pub start_time_ns: i64,
}

#[derive(Default)]
pub struct RecordingMlflowClient {
    pub calls: Arc<Mutex<Vec<String>>>,
}

impl RecordingMlflowClient {
    fn record(&self, call: String) -> Result<(), MlflowError> {
        self.calls
            .lock()
            .map_err(|_| MlflowError::Operation("recording lock poisoned".to_owned()))?
            .push(call);
        Ok(())
    }
}

impl MlflowClient for RecordingMlflowClient {
    fn active_span(&self) -> Result<Option<Span>, MlflowError> {
        Ok(None)
    }
    fn start_trace(&self, request: StartRequest) -> Result<Span, MlflowError> {
        self.record(format!("trace:{}", request.name))?;
        Ok(Span {
            request_id: "trace".to_owned(),
            span_id: "span".to_owned(),
            parent_id: None,
        })
    }
    fn start_span(
        &self,
        request: StartRequest,
        trace_id: &str,
        parent_id: &str,
    ) -> Result<Span, MlflowError> {
        self.record(format!("span:{}:{}:{}", request.name, trace_id, parent_id))?;
        Ok(Span {
            request_id: trace_id.to_owned(),
            span_id: "span".to_owned(),
            parent_id: Some(parent_id.to_owned()),
        })
    }
    fn add_event(&self, _span: &Span, event: MlflowEvent) -> Result<(), MlflowError> {
        self.record(event.name)
    }
    fn end_trace(
        &self,
        _span: &Span,
        _outputs: &Value,
        _status: Status,
        _end_time_ns: i64,
    ) -> Result<(), MlflowError> {
        self.record("end_trace".to_owned())
    }
    fn end_span(
        &self,
        _span: &Span,
        _outputs: &Value,
        _status: Status,
        _end_time_ns: i64,
    ) -> Result<(), MlflowError> {
        self.record("end_span".to_owned())
    }
}
