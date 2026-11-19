use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_foundry::passthrough::logging::FoundryPassthroughLogging;
use betterlite_foundry::passthrough::ocr::NativeOcrPassthrough;
use betterlite_ocr::ocr_error::OcrError;
use betterlite_ocr::response::OCRResponse;
use betterlite_openai::passthrough::transformation::types::{PassthroughLogging, RelayCallType};
use serde_json::{Value, json};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use super::input::OcrStep;

#[derive(Debug, Default)]
pub(super) struct RecordingLogging {
    pub(super) call_type: Option<String>,
}

impl PassthroughLogging for RecordingLogging {
    fn set_call_type(&mut self, call_type: RelayCallType) {
        self.call_type = Some(call_type.as_str().to_owned());
    }
}

impl FoundryPassthroughLogging for RecordingLogging {
    fn set_ocr_call_type(&mut self) {
        self.call_type = Some(String::from("aocr"));
    }
}

struct RecordedOcrCall {
    model: String,
    endpoint: String,
    body: String,
}

pub(super) struct ScriptedOcr {
    response: Option<OCRResponse>,
    malformed: bool,
    calls: Mutex<Vec<RecordedOcrCall>>,
}

impl ScriptedOcr {
    pub(super) fn calls_json(&self) -> Vec<Value> {
        let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls
            .iter()
            .map(|call| json!({"model": call.model, "endpoint": call.endpoint, "body": call.body}))
            .collect()
    }
}

impl NativeOcrPassthrough for ScriptedOcr {
    fn passthrough_response(
        &self,
        model: &str,
        endpoint: &str,
        body: &[u8],
    ) -> Result<Option<OCRResponse>, OcrError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedOcrCall {
                model: model.to_owned(),
                endpoint: endpoint.to_owned(),
                body: String::from_utf8_lossy(body).into_owned(),
            });
        if self.malformed {
            return Err(OcrError::InvalidRequest(String::from("malformed OCR body")));
        }
        Ok(self.response.clone())
    }
}

pub(super) fn scripted_ocr(step: &OcrStep) -> Arc<ScriptedOcr> {
    let response = step
        .response
        .as_ref()
        .and_then(|text| serde_json::from_str::<OCRResponse>(text).ok());
    let malformed = step.malformed || (step.response.is_some() && response.is_none());
    Arc::new(ScriptedOcr {
        response,
        malformed,
        calls: Mutex::new(Vec::new()),
    })
}

#[derive(Clone, Default)]
pub(super) struct Warnings(Arc<AtomicBool>);

impl<Inner: Subscriber> Layer<Inner> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        self.0.store(true, Ordering::Relaxed);
    }
}

// only the OCR warning is a WARN event on this path, so one raised flag is enough
pub(super) fn capture_warnings<T>(run: impl FnOnce() -> T) -> (T, bool) {
    let warnings = Warnings::default();
    let subscriber = tracing_subscriber::registry().with(warnings.clone());
    let value = tracing::subscriber::with_default(subscriber, run);
    (value, warnings.0.load(Ordering::Relaxed))
}
