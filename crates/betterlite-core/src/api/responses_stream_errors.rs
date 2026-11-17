use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use serde_json::Number;
use uuid::Uuid;

use self::error_codes::response_error_code;
use self::failure::{
    FailureCause, FailureDetails, committed_text, failure_details, original_failure,
};

mod error_codes;
pub mod failure;
mod tests;

#[derive(Clone, Debug, serde::Deserialize)]
struct StreamEvent {
    #[serde(default, rename = "type")]
    event_type: Option<String>,
    #[serde(default)]
    sequence_number: Option<i64>,
    #[serde(default)]
    response: Option<ResponseIdentity>,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct ResponseIdentity {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    created_at: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
pub enum StreamEventError {
    #[error("a response stream event is not a readable mapping: {0}")]
    Chunk(#[from] serde_json::Error),
}

pub struct ResponsesStreamErrorState {
    pub response_id: Option<String>,
    pub model: Option<String>,
    pub created_at: Option<i64>,
    pub sequence_number: i64,
    pub terminal_emitted: bool,
    pending_event: Option<StreamEvent>,
    now: Arc<dyn Fn() -> SystemTime + Send + Sync>,
    next_id: Arc<dyn Fn() -> Uuid + Send + Sync>,
    redactor: Option<Arc<Redactor>>,
}

impl ResponsesStreamErrorState {
    #[must_use]
    pub fn new(
        now: Arc<dyn Fn() -> SystemTime + Send + Sync>,
        next_id: Arc<dyn Fn() -> Uuid + Send + Sync>,
        redactor: Option<Arc<Redactor>>,
    ) -> Self {
        Self {
            response_id: None,
            model: None,
            created_at: None,
            sequence_number: -1,
            terminal_emitted: false,
            pending_event: None,
            now,
            next_id,
            redactor,
        }
    }

    pub fn observe_chunk(&mut self, chunk: &Json) -> Result<(), StreamEventError> {
        let Json::Object(_) = chunk else {
            self.pending_event = None;
            return Ok(());
        };
        let text = serde_json::to_string(chunk)?;
        self.pending_event = Some(serde_json::from_str(&text)?);
        Ok(())
    }

    // The frame goes back exactly as it came, so a caller may pass text or bytes.
    #[must_use]
    pub fn mark_emitted<T>(&mut self, frame: T) -> T {
        let Some(event) = self.pending_event.clone() else {
            return frame;
        };
        if let Some(sequence_number) = event.sequence_number {
            self.sequence_number = self.sequence_number.max(sequence_number);
        }
        if let Some(identity) = &event.response {
            self.response_id = committed_text(identity.id.clone(), self.response_id.clone());
            self.model = committed_text(identity.model.clone(), self.model.clone());
            if identity.created_at.is_some() {
                self.created_at = identity.created_at;
            }
        }
        if matches!(
            event.event_type.as_deref(),
            Some("response.completed" | "response.failed" | "response.incomplete")
        ) {
            self.terminal_emitted = true;
        }
        frame
    }

    #[must_use]
    pub fn format_failure(&mut self, failure: &FailureCause) -> Option<String> {
        if self.terminal_emitted {
            return None;
        }
        let original = original_failure(failure);
        let details = failure_details(original);
        let payload = self.failure_payload(&details, original);
        self.terminal_emitted = true;
        Some(format!("event: response.failed\ndata: {payload}\n\n"))
    }

    fn failure_payload(&self, details: &FailureDetails, original: &FailureCause) -> String {
        let mut event = IndexMap::new();
        event.insert("type".to_owned(), Json::Text("response.failed".to_owned()));
        event.insert(
            "response".to_owned(),
            self.failure_response(details, original),
        );
        event.insert(
            "sequence_number".to_owned(),
            Json::Number(Number::from(self.sequence_number.saturating_add(1))),
        );
        // A Json tree of strings, integers and lists always serializes, so the empty fallback is unreachable.
        serde_json::to_string(&Json::Object(event)).unwrap_or_default()
    }

    fn failure_response(&self, details: &FailureDetails, original: &FailureCause) -> Json {
        let mut error = IndexMap::new();
        error.insert("code".to_owned(), Json::Text(response_error_code(details)));
        error.insert(
            "message".to_owned(),
            Json::Text(self.client_message(details, original)),
        );
        let mut response = IndexMap::new();
        response.insert("id".to_owned(), Json::Text(self.failure_id()));
        response.insert(
            "created_at".to_owned(),
            Json::Number(Number::from(self.failure_created_at())),
        );
        response.insert("error".to_owned(), Json::Object(error));
        if let Some(model) = &self.model {
            response.insert("model".to_owned(), Json::Text(model.clone()));
        }
        response.insert("object".to_owned(), Json::Text("response".to_owned()));
        response.insert("output".to_owned(), Json::List(Vec::new()));
        response.insert("status".to_owned(), Json::Text("failed".to_owned()));
        Json::Object(response)
    }

    fn failure_id(&self) -> String {
        match self.response_id.as_deref() {
            Some(id) if !id.is_empty() => id.to_owned(),
            _ => format!("resp_{}", (self.next_id)().simple()),
        }
    }

    fn failure_created_at(&self) -> i64 {
        match self.created_at {
            Some(created_at) => created_at,
            None => epoch_seconds((self.now)()),
        }
    }

    fn client_message(&self, details: &FailureDetails, original: &FailureCause) -> String {
        let message = match details.message.as_deref() {
            Some(message) if !message.is_empty() => message.to_owned(),
            _ => original.text.clone(),
        };
        let Some(redactor) = &self.redactor else {
            return message;
        };
        redactor.redact_internal_details(&message)
    }
}

fn epoch_seconds(now: SystemTime) -> i64 {
    match now.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_secs()).unwrap_or(i64::MAX),
        Err(before) => {
            i64::try_from(before.duration().as_secs()).map_or(i64::MAX, i64::saturating_neg)
        }
    }
}
