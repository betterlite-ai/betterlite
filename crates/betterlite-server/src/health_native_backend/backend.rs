use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_logging::logging_state::correlation::CorrelationContext;
use betterlite_models::call_helpers::headers::provider_response_headers;
use betterlite_models::controls::cheapest::pick_cheapest_chat_models;
use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::PriceLookup;
use betterlite_models::provider_model_sets::ProviderModelSets;
use betterlite_sdk::call_setup::RequestMap;
use betterlite_sdk::call_setup::correlation::CallOutcome;
use betterlite_sdk::call_wrapper::error::CallError;
use betterlite_sdk::call_wrapper::logging::AsyncSuccessLog;
use betterlite_sdk::call_wrapper::state::LoggingHandle;
use betterlite_sdk::call_wrapper::target::CallTarget;
use secrecy::ExposeSecret as _;

use super::deferred::not_built;
use super::seams::{
    BatchProbeCall, EmbeddingProbeCall, EvaluationProbeCall, ImageEditProbeCall,
    ImageGenerationProbeCall, OcrProbeCall, RerankProbeCall, ResponsesProbeCall, SpeechProbeCall,
    TextCompletionProbeCall, TranscriptionProbeCall,
};
use crate::health_probe_modes::backend::{CheapestChatModels, HealthProbeBackend};
use crate::health_probe_modes::calls::{
    AudioTranscriptionCall, EvaluationCall, HealthProbeReply, ImageEditCall, OcrCall, ProbeCall,
    RealtimeCall,
};
use crate::health_probe_modes::error::HealthProbeError;

const HIDDEN_PARAMS_KEY: &str = "_hidden_params";
const API_KEY_KEY: &str = "api_key";

// ahealth_check's real operations: chat through B5656's CallTarget, one injectable seam per other mode
pub struct NativeHealthBackend {
    pub chat: Arc<dyn CallTarget>,
    pub completion: Arc<dyn TextCompletionProbeCall>,
    pub embedding: Arc<dyn EmbeddingProbeCall>,
    pub audio_speech: Arc<dyn SpeechProbeCall>,
    pub audio_transcription: Arc<dyn TranscriptionProbeCall>,
    pub image_generation: Arc<dyn ImageGenerationProbeCall>,
    pub image_edit: Arc<dyn ImageEditProbeCall>,
    pub rerank: Arc<dyn RerankProbeCall>,
    pub batch: Arc<dyn BatchProbeCall>,
    pub responses: Arc<dyn ResponsesProbeCall>,
    pub ocr: Arc<dyn OcrProbeCall>,
    pub evaluation: Arc<dyn EvaluationProbeCall>,
    pub prices: Arc<dyn PriceLookup + Send + Sync>,
    pub model_sets: Arc<ProviderModelSets>,
}

impl CheapestChatModels for NativeHealthBackend {
    fn cheapest_chat_models(&self, custom_llm_provider: &str, limit: usize) -> Vec<String> {
        let known = self
            .model_sets
            .models_by_provider
            .get(custom_llm_provider)
            .map(|models| models.iter().cloned().collect::<Vec<String>>());
        let prices: &dyn PriceLookup = self.prices.as_ref();
        pick_cheapest_chat_models(custom_llm_provider, limit, known.as_deref(), prices)
    }
}

#[async_trait]
impl HealthProbeBackend for NativeHealthBackend {
    async fn chat(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        let request = chat_request(&call);
        let logging = chat_logging(&call);
        let outcome = self
            .chat
            .call(request, logging.as_ref())
            .await
            .map_err(chat_failure)?;
        Ok(reply_from_outcome(outcome))
    }

    async fn completion(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.completion.complete(call).await
    }

    async fn embedding(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.embedding.embed(call).await
    }

    async fn audio_speech(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.audio_speech.speak(call).await
    }

    async fn audio_transcription(
        &self,
        call: AudioTranscriptionCall,
    ) -> Result<HealthProbeReply, HealthProbeError> {
        self.audio_transcription.transcribe(call).await
    }

    async fn image_generation(
        &self,
        call: ProbeCall,
    ) -> Result<HealthProbeReply, HealthProbeError> {
        self.image_generation.generate(call).await
    }

    async fn image_edit(&self, call: ImageEditCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.image_edit.edit(call).await
    }

    async fn video_generation(
        &self,
        _call: ProbeCall,
    ) -> Result<HealthProbeReply, HealthProbeError> {
        Err(not_built("video_generation"))
    }

    async fn rerank(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.rerank.rerank(call).await
    }

    async fn realtime(&self, _call: RealtimeCall) -> Result<HealthProbeReply, HealthProbeError> {
        Err(not_built("realtime"))
    }

    async fn batch(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.batch.list_batches(call).await
    }

    async fn responses(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.responses.create(call).await
    }

    async fn ocr(&self, call: OcrCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.ocr.ocr(call).await
    }

    async fn evaluation(&self, call: EvaluationCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.evaluation.evaluate(call).await
    }
}

// the acompletion kwargs: B4440's parameters plus the api key, which travels as its own keyword
fn chat_request(call: &ProbeCall) -> RequestMap {
    let mut request = call.parameters.clone();
    if let Some(key) = &call.api_key {
        let value = Json::Text(key.expose_secret().to_owned());
        request.insert(String::from(API_KEY_KEY), value);
    }
    request
}

// the logging object reaches the call as a handle over the same state, so the entry still reads its raw request
fn chat_logging(call: &ProbeCall) -> Option<LoggingHandle> {
    let state = call.logging.as_ref()?.logging_state()?;
    Some(LoggingHandle {
        state,
        correlation: Arc::new(Mutex::new(CorrelationContext::default())),
        deferred: Arc::new(Mutex::new(None::<AsyncSuccessLog>)),
    })
}

fn chat_failure(error: CallError) -> HealthProbeError {
    HealthProbeError::Backend {
        class: error.class.clone(),
        message: error.to_string(),
        source: Box::new(error),
    }
}

// ahealth_check reads _hidden_params.headers off the response; a stream carries none
fn reply_from_outcome(outcome: CallOutcome) -> HealthProbeReply {
    let CallOutcome::Response(body) = outcome else {
        return HealthProbeReply::default();
    };
    HealthProbeReply {
        hidden_headers: response_headers(&body),
    }
}

fn response_headers(body: &Json) -> BTreeMap<String, String> {
    let Json::Object(fields) = body else {
        return BTreeMap::new();
    };
    let Some(hidden) = fields.get(HIDDEN_PARAMS_KEY) else {
        return BTreeMap::new();
    };
    let Some(headers) = provider_response_headers(Some(hidden)) else {
        return BTreeMap::new();
    };
    headers.into_iter().collect()
}
