use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_models::decisions::Json;
use betterlite_ocr::ocr_document::OcrDocument;
use betterlite_server::health_probe_modes::backend::{
    CheapestChatModels, HealthProbeBackend, OcrHealthDocuments,
};
use betterlite_server::health_probe_modes::calls::{
    AudioTranscriptionCall, EvaluationCall, HealthProbeReply, ImageEditCall, OcrCall, ProbeCall,
    RealtimeCall,
};
use betterlite_server::health_probe_modes::error::HealthProbeError;
use betterlite_server::health_probe_modes::logging::HealthProbeLogging;
use serde::Serialize;
use serde_json::{Value, json};

use super::RawRequest;

const RAW_REQUEST_KEY: &str = "raw_request_typed_dict";

// one probe call as the case observes it: which operation ran, with what arguments, through which logging object
pub(super) struct ProbeRecord {
    pub(super) name: &'static str,
    pub(super) arguments: Value,
    pub(super) logging: Option<Arc<dyn HealthProbeLogging>>,
}

// the backend the cases script: it records the call, writes the raw request and answers with the scripted headers
pub(super) struct ProbeRecorder {
    pub(super) headers: Option<BTreeMap<String, String>>,
    pub(super) failure: Option<String>,
    pub(super) raw_request: Option<RawRequest>,
    pub(super) call: Mutex<Option<ProbeRecord>>,
}

impl ProbeRecorder {
    pub(super) fn new(
        headers: Option<BTreeMap<String, String>>,
        failure: Option<String>,
        raw_request: Option<RawRequest>,
    ) -> Self {
        Self {
            headers,
            failure,
            raw_request,
            call: Mutex::new(None),
        }
    }

    fn note<T>(
        &self,
        name: &'static str,
        arguments: &T,
        logging: Option<Arc<dyn HealthProbeLogging>>,
    ) -> Result<HealthProbeReply, HealthProbeError>
    where
        T: Serialize,
    {
        let recorded = serde_json::to_value(arguments).unwrap_or(Value::Null);
        self.write_raw_request(logging.as_ref());
        let failure = self.failure.clone();
        *self.call.lock().unwrap_or_else(PoisonError::into_inner) = Some(ProbeRecord {
            name,
            arguments: recorded,
            logging,
        });
        if let Some(message) = failure {
            return Err(backend_error(&message));
        }
        Ok(HealthProbeReply {
            hidden_headers: self.headers.clone().unwrap_or_default(),
        })
    }

    // the provider call writes what LiteLLM's transport would have logged for the failure result
    fn write_raw_request(&self, logging: Option<&Arc<dyn HealthProbeLogging>>) {
        let Some(raw) = self.raw_request.as_ref() else {
            return;
        };
        let Some(object) = logging else {
            return;
        };
        let Some(state) = object.logging_state() else {
            return;
        };
        let value = serde_json::to_value(raw).unwrap_or(Value::Null);
        let Ok(record) = serde_json::from_value::<Json>(value) else {
            return;
        };
        state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .model_call_details
            .insert(RAW_REQUEST_KEY.to_owned(), record);
    }
}

#[async_trait]
impl HealthProbeBackend for ProbeRecorder {
    async fn chat(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("acompletion", &call.parameters, call.logging)
    }

    async fn completion(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("atext_completion", &call.parameters, call.logging)
    }

    async fn embedding(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("aembedding", &call.parameters, call.logging)
    }

    async fn audio_speech(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("aspeech", &call.parameters, call.logging)
    }

    async fn audio_transcription(
        &self,
        call: AudioTranscriptionCall,
    ) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("atranscription", &call.parameters, call.logging)
    }

    async fn image_generation(
        &self,
        call: ProbeCall,
    ) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("aimage_generation", &call.parameters, call.logging)
    }

    async fn image_edit(&self, call: ImageEditCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("aimage_edit", &call.parameters, call.logging)
    }

    async fn video_generation(
        &self,
        call: ProbeCall,
    ) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("avideo_generation", &call.parameters, call.logging)
    }

    async fn rerank(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("arerank", &call.parameters, call.logging)
    }

    async fn realtime(&self, call: RealtimeCall) -> Result<HealthProbeReply, HealthProbeError> {
        let arguments = json!({
            "model": call.model,
            "custom_llm_provider": call.custom_llm_provider,
            "api_base": call.api_base,
            "api_version": call.api_version,
            "model_params": &call.model_params,
        });
        self.note("realtime_health_check", &arguments, call.logging)
    }

    async fn batch(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("alist_batches", &call.parameters, call.logging)
    }

    async fn responses(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("aresponses", &call.parameters, call.logging)
    }

    async fn ocr(&self, call: OcrCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("aocr", &call.parameters, call.logging)
    }

    async fn evaluation(&self, call: EvaluationCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("asystemone", &call.params, call.logging)
    }
}

pub(super) struct ScriptedCheapest {
    pub(super) models: Vec<String>,
}

impl CheapestChatModels for ScriptedCheapest {
    fn cheapest_chat_models(&self, _custom_llm_provider: &str, limit: usize) -> Vec<String> {
        self.models.iter().take(limit).cloned().collect()
    }
}

// no case probes the ocr mode, so the document lookup only has to answer with a valid document
pub(super) struct HealthCheckDocuments;

impl OcrHealthDocuments for HealthCheckDocuments {
    fn health_check_document(
        &self,
        _model: &str,
        _custom_llm_provider: &str,
    ) -> Result<OcrDocument, HealthProbeError> {
        Ok(OcrDocument::DocumentUrl {
            document_url: "data:application/pdf;base64,aGVhbHRo".to_owned(),
            extra: BTreeMap::new(),
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

fn backend_error(message: &str) -> HealthProbeError {
    let context = error_context(message);
    HealthProbeError::Backend {
        class: ErrorClass::Unclassified(Box::new(context)),
        message: message.to_owned(),
        source: Box::new(io::Error::other(message.to_owned())),
    }
}
