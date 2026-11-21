use super::Failure;
use async_trait::async_trait;
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_logging::logging_state::{LoggingState, LoggingStateError};
use betterlite_ocr::ocr_document::OcrDocument;
use betterlite_server::health_probe_modes::backend::{
    CheapestChatModels, HealthProbeBackend, OcrHealthDocuments,
};
use betterlite_server::health_probe_modes::calls::{
    AudioTranscriptionCall, EvaluationCall, HealthProbeReply, ImageEditCall, OcrCall, ProbeCall,
    RealtimeCall,
};
use betterlite_server::health_probe_modes::error::HealthProbeError;
use betterlite_server::health_probe_modes::logging::{HealthProbeLogging, ProbeLoggingUpdate};
use serde_json::{Value, json};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
pub(super) struct Recorder {
    pub(super) calls: Mutex<Vec<Value>>,
    failure: Option<Failure>,
}
impl Recorder {
    pub(super) fn new(failure: Option<&Failure>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failure: failure.cloned(),
        }
    }
    fn note(&self, name: &str, arguments: Value) -> Result<HealthProbeReply, HealthProbeError> {
        let entry = json!({"name": name, "arguments": arguments});
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
        match &self.failure {
            None => Ok(HealthProbeReply::default()),
            Some(failure) => Err(backend_error(failure)),
        }
    }
}
macro_rules! record_calls {
    ($(($method:ident, $call_type:ty, $name:literal)),+ $(,)?) => {
        #[async_trait]
        impl HealthProbeBackend for Recorder {
            $(
                async fn $method(&self, call: $call_type) -> Result<HealthProbeReply, HealthProbeError> {
                    let arguments = serde_json::to_value(&call.parameters).unwrap_or(Value::Null);
                    self.note($name, arguments)
                }
            )+
        }
    };
}
record_calls!(
    (chat, ProbeCall, "acompletion"),
    (completion, ProbeCall, "atext_completion"),
    (embedding, ProbeCall, "aembedding"),
    (audio_speech, ProbeCall, "aspeech"),
    (
        audio_transcription,
        AudioTranscriptionCall,
        "atranscription"
    ),
    (image_generation, ProbeCall, "aimage_generation"),
    (video_generation, ProbeCall, "avideo_generation"),
    (rerank, ProbeCall, "arerank"),
    (responses, ProbeCall, "aresponses"),
    (batch, ProbeCall, "alist_batches"),
    (ocr, OcrCall, "aocr"),
);
#[async_trait]
impl HealthProbeBackend for Recorder {
    async fn image_edit(&self, call: ImageEditCall) -> Result<HealthProbeReply, HealthProbeError> {
        let mut arguments = serde_json::to_value(&call.parameters).unwrap_or(Value::Null);
        if let Value::Object(entries) = &mut arguments {
            entries.insert("prompt".to_owned(), Value::String(call.prompt.to_owned()));
        }
        self.note("aimage_edit", arguments)
    }
    async fn realtime(&self, call: RealtimeCall) -> Result<HealthProbeReply, HealthProbeError> {
        let arguments = json!({
            "model": call.model,
            "custom_llm_provider": call.custom_llm_provider,
            "api_base": call.api_base,
            "api_version": call.api_version,
            "model_params": serde_json::to_value(&call.model_params).unwrap_or(Value::Null),
        });
        self.note("realtime_health_check", arguments)
    }
    async fn evaluation(&self, call: EvaluationCall) -> Result<HealthProbeReply, HealthProbeError> {
        let arguments = serde_json::to_value(&call.params).unwrap_or(Value::Null);
        self.note("asystemone", arguments)
    }
}
#[derive(Default)]
pub(super) struct UpdateRecorder {
    pub(super) update: Mutex<Option<Value>>,
}
impl HealthProbeLogging for UpdateRecorder {
    fn update_from_kwargs(&self, update: ProbeLoggingUpdate) -> Result<(), LoggingStateError> {
        let kwargs = serde_json::to_value(&update.kwargs).unwrap_or(Value::Null);
        let optional_params = serde_json::to_value(&update.optional_params).unwrap_or(Value::Null);
        let litellm_params = update
            .litellm_params
            .as_ref()
            .map(|params| serde_json::to_value(params).unwrap_or(Value::Null));
        let recorded = json!({
            "kwargs": kwargs,
            "model": update.model,
            "user": update.user,
            "optional_params": optional_params,
            "litellm_params": litellm_params,
        });
        *self.update.lock().unwrap_or_else(PoisonError::into_inner) = Some(recorded);
        Ok(())
    }
    fn logging_state(&self) -> Option<Arc<Mutex<LoggingState>>> {
        None
    }
}
pub(super) struct FixedCheapest {
    pub(super) models: Vec<String>,
}
impl CheapestChatModels for FixedCheapest {
    fn cheapest_chat_models(&self, _custom_llm_provider: &str, n: usize) -> Vec<String> {
        self.models.iter().take(n).cloned().collect()
    }
}
pub(super) struct MissingDocuments;
impl OcrHealthDocuments for MissingDocuments {
    fn health_check_document(
        &self,
        model: &str,
        custom_llm_provider: &str,
    ) -> Result<OcrDocument, HealthProbeError> {
        Err(HealthProbeError::OcrDocumentUnavailable {
            model: model.to_owned(),
            custom_llm_provider: custom_llm_provider.to_owned(),
        })
    }
}
fn error_context(message: &str) -> ErrorContext {
    ErrorContext {
        message: message.to_owned(),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}
fn error_class(failure: &Failure) -> ErrorClass {
    let context = error_context(&failure.message);
    match failure.class_name.as_str() {
        "bad_request" => ErrorClass::BadRequest(Box::new(ResponseDetails::from(context))),
        "context_window" => {
            ErrorClass::ContextWindowExceeded(Box::new(ResponseDetails::from(context)))
        }
        "content_policy" => {
            ErrorClass::ContentPolicyViolation(Box::new(ResponseDetails::from(context)))
        }
        "unsupported_params" => ErrorClass::UnsupportedParams(Box::new(context)),
        _ => ErrorClass::Unclassified(Box::new(context)),
    }
}
fn backend_error(failure: &Failure) -> HealthProbeError {
    HealthProbeError::Backend {
        class: error_class(failure),
        message: failure.message.clone(),
        source: Box::new(io::Error::other(failure.message.clone())),
    }
}
