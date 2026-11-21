use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_logging::logging_state::{LoggingState, LoggingStateError};
use betterlite_ocr::ocr_document::OcrDocument;

use crate::health_probe_modes::backend::{
    CheapestChatModels, HealthProbeBackend, OcrHealthDocuments,
};
use crate::health_probe_modes::calls::{
    AudioTranscriptionCall, EvaluationCall, HealthProbeReply, ImageEditCall, OcrCall, ProbeCall,
    RealtimeCall,
};
use crate::health_probe_modes::error::HealthProbeError;
use crate::health_probe_modes::logging::{HealthProbeLogging, ProbeLoggingUpdate};
use crate::health_probe_modes::params::HealthProbeParams;
use crate::health_probe_modes::plan::{HealthProbeArguments, plan_health_probe};
use crate::health_probe_modes::run::run_health_probe;

pub(super) const DOCUMENT_URL: &str = "data:application/pdf;base64,aGVhbHRo";
pub(super) const IMAGE_BYTES: usize = 2508;

enum RecordedCall {
    Chat(ProbeCall),
    Completion(ProbeCall),
    Embedding(ProbeCall),
    AudioSpeech(ProbeCall),
    AudioTranscription(AudioTranscriptionCall),
    ImageGeneration(ProbeCall),
    ImageEdit(ImageEditCall),
    VideoGeneration(ProbeCall),
    Rerank(ProbeCall),
    Realtime(RealtimeCall),
    Batch(ProbeCall),
    Responses(ProbeCall),
    Ocr(OcrCall),
    Evaluation(EvaluationCall),
}

fn recorded_mode(call: &RecordedCall) -> &'static str {
    match call {
        RecordedCall::Chat(_) => "chat",
        RecordedCall::Completion(_) => "completion",
        RecordedCall::Embedding(_) => "embedding",
        RecordedCall::AudioSpeech(_) => "audio_speech",
        RecordedCall::AudioTranscription(_) => "audio_transcription",
        RecordedCall::ImageGeneration(_) => "image_generation",
        RecordedCall::ImageEdit(_) => "image_edit",
        RecordedCall::VideoGeneration(_) => "video_generation",
        RecordedCall::Rerank(_) => "rerank",
        RecordedCall::Realtime(_) => "realtime",
        RecordedCall::Batch(_) => "batch",
        RecordedCall::Responses(_) => "responses",
        RecordedCall::Ocr(_) => "ocr",
        RecordedCall::Evaluation(_) => "evaluation",
    }
}

struct FakeFailure {
    class: ErrorClass,
    message: String,
}

pub(super) struct FakeBackend {
    calls: Mutex<Vec<RecordedCall>>,
    pub(super) headers: BTreeMap<String, String>,
    image_edit_failure: Mutex<Option<FakeFailure>>,
    events: Arc<Mutex<Vec<&'static str>>>,
}

impl FakeBackend {
    pub(super) fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            headers: BTreeMap::new(),
            image_edit_failure: Mutex::new(None),
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn events(&self) -> Arc<Mutex<Vec<&'static str>>> {
        Arc::clone(&self.events)
    }

    fn record(&self, call: RecordedCall) -> HealthProbeReply {
        self.events.lock().unwrap().push("call");
        self.calls.lock().unwrap().push(call);
        HealthProbeReply {
            hidden_headers: self.headers.clone(),
        }
    }

    pub(super) fn fail_image_edit(&self, class: ErrorClass, message: &str) {
        let failure = FakeFailure {
            class,
            message: message.to_owned(),
        };
        *self.image_edit_failure.lock().unwrap() = Some(failure);
    }

    fn take_last(&self) -> RecordedCall {
        self.calls.lock().unwrap().pop().unwrap()
    }

    pub(super) fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    pub(super) fn take_mode(&self) -> &'static str {
        recorded_mode(&self.take_last())
    }

    pub(super) fn take_chat(&self) -> ProbeCall {
        let RecordedCall::Chat(call) = self.take_last() else {
            panic!("the mode sends a chat call");
        };
        call
    }

    pub(super) fn take_completion(&self) -> ProbeCall {
        let RecordedCall::Completion(call) = self.take_last() else {
            panic!("the mode sends a text completion call");
        };
        call
    }

    pub(super) fn take_embedding(&self) -> ProbeCall {
        let RecordedCall::Embedding(call) = self.take_last() else {
            panic!("the mode sends an embedding call");
        };
        call
    }

    pub(super) fn take_audio_speech(&self) -> ProbeCall {
        let RecordedCall::AudioSpeech(call) = self.take_last() else {
            panic!("the mode sends a speech call");
        };
        call
    }

    pub(super) fn take_audio_transcription(&self) -> AudioTranscriptionCall {
        let RecordedCall::AudioTranscription(call) = self.take_last() else {
            panic!("the mode sends a transcription call");
        };
        call
    }

    pub(super) fn take_image_generation(&self) -> ProbeCall {
        let RecordedCall::ImageGeneration(call) = self.take_last() else {
            panic!("the mode sends an image generation call");
        };
        call
    }

    pub(super) fn take_image_edit(&self) -> ImageEditCall {
        let RecordedCall::ImageEdit(call) = self.take_last() else {
            panic!("the mode sends an image edit call");
        };
        call
    }

    pub(super) fn take_video_generation(&self) -> ProbeCall {
        let RecordedCall::VideoGeneration(call) = self.take_last() else {
            panic!("the mode sends a video generation call");
        };
        call
    }

    pub(super) fn take_rerank(&self) -> ProbeCall {
        let RecordedCall::Rerank(call) = self.take_last() else {
            panic!("the mode sends a rerank call");
        };
        call
    }

    pub(super) fn take_realtime(&self) -> RealtimeCall {
        let RecordedCall::Realtime(call) = self.take_last() else {
            panic!("the mode sends a realtime call");
        };
        call
    }

    pub(super) fn take_batch(&self) -> ProbeCall {
        let RecordedCall::Batch(call) = self.take_last() else {
            panic!("the mode sends a list batches call");
        };
        call
    }

    pub(super) fn take_responses(&self) -> ProbeCall {
        let RecordedCall::Responses(call) = self.take_last() else {
            panic!("the mode sends a responses call");
        };
        call
    }

    pub(super) fn take_ocr(&self) -> OcrCall {
        let RecordedCall::Ocr(call) = self.take_last() else {
            panic!("the mode sends an ocr call");
        };
        call
    }

    pub(super) fn take_evaluation(&self) -> EvaluationCall {
        let RecordedCall::Evaluation(call) = self.take_last() else {
            panic!("the mode sends an evaluation call");
        };
        call
    }
}

#[async_trait]
impl HealthProbeBackend for FakeBackend {
    async fn chat(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        Ok(self.record(RecordedCall::Chat(call)))
    }

    async fn completion(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        Ok(self.record(RecordedCall::Completion(call)))
    }

    async fn embedding(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        Ok(self.record(RecordedCall::Embedding(call)))
    }

    async fn audio_speech(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        Ok(self.record(RecordedCall::AudioSpeech(call)))
    }

    async fn audio_transcription(
        &self,
        call: AudioTranscriptionCall,
    ) -> Result<HealthProbeReply, HealthProbeError> {
        Ok(self.record(RecordedCall::AudioTranscription(call)))
    }

    async fn image_generation(
        &self,
        call: ProbeCall,
    ) -> Result<HealthProbeReply, HealthProbeError> {
        Ok(self.record(RecordedCall::ImageGeneration(call)))
    }

    async fn image_edit(&self, call: ImageEditCall) -> Result<HealthProbeReply, HealthProbeError> {
        let reply = self.record(RecordedCall::ImageEdit(call));
        let failure = self.image_edit_failure.lock().unwrap().take();
        match failure {
            None => Ok(reply),
            Some(failure) => {
                let source = std::io::Error::other(failure.message.clone());
                Err(HealthProbeError::Backend {
                    class: failure.class,
                    message: failure.message,
                    source: Box::new(source),
                })
            }
        }
    }

    async fn video_generation(
        &self,
        call: ProbeCall,
    ) -> Result<HealthProbeReply, HealthProbeError> {
        Ok(self.record(RecordedCall::VideoGeneration(call)))
    }

    async fn rerank(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        Ok(self.record(RecordedCall::Rerank(call)))
    }

    async fn realtime(&self, call: RealtimeCall) -> Result<HealthProbeReply, HealthProbeError> {
        Ok(self.record(RecordedCall::Realtime(call)))
    }

    async fn batch(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        Ok(self.record(RecordedCall::Batch(call)))
    }

    async fn responses(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        Ok(self.record(RecordedCall::Responses(call)))
    }

    async fn ocr(&self, call: OcrCall) -> Result<HealthProbeReply, HealthProbeError> {
        Ok(self.record(RecordedCall::Ocr(call)))
    }

    async fn evaluation(&self, call: EvaluationCall) -> Result<HealthProbeReply, HealthProbeError> {
        Ok(self.record(RecordedCall::Evaluation(call)))
    }
}

pub(super) struct FakeCheapestModels {
    models: Vec<String>,
    pub(super) request: Mutex<Option<(String, usize)>>,
}

impl FakeCheapestModels {
    pub(super) fn new(models: &[&str]) -> Self {
        let models = models.iter().map(|model| String::from(*model)).collect();
        Self {
            models,
            request: Mutex::new(None),
        }
    }
}

impl CheapestChatModels for FakeCheapestModels {
    fn cheapest_chat_models(&self, custom_llm_provider: &str, n: usize) -> Vec<String> {
        *self.request.lock().unwrap() = Some((custom_llm_provider.to_owned(), n));
        self.models.clone()
    }
}

pub(super) struct FakeDocuments {
    pub(super) missing: bool,
}

impl OcrHealthDocuments for FakeDocuments {
    fn health_check_document(
        &self,
        model: &str,
        custom_llm_provider: &str,
    ) -> Result<OcrDocument, HealthProbeError> {
        if self.missing {
            return Err(HealthProbeError::OcrDocumentUnavailable {
                model: model.to_owned(),
                custom_llm_provider: custom_llm_provider.to_owned(),
            });
        }
        Ok(OcrDocument::DocumentUrl {
            document_url: DOCUMENT_URL.to_owned(),
            extra: BTreeMap::new(),
        })
    }
}

pub(super) struct FakeLogging {
    updates: Mutex<Vec<ProbeLoggingUpdate>>,
    pub(super) events: Arc<Mutex<Vec<&'static str>>>,
    failing: bool,
}

impl FakeLogging {
    pub(super) fn new(events: Arc<Mutex<Vec<&'static str>>>, failing: bool) -> Self {
        Self {
            updates: Mutex::new(Vec::new()),
            events,
            failing,
        }
    }

    pub(super) fn take_updates(&self) -> Vec<ProbeLoggingUpdate> {
        std::mem::take(&mut *self.updates.lock().unwrap())
    }
}

impl HealthProbeLogging for FakeLogging {
    fn update_from_kwargs(&self, update: ProbeLoggingUpdate) -> Result<(), LoggingStateError> {
        self.events.lock().unwrap().push("logging");
        if self.failing {
            return Err(LoggingStateError::FieldType { field: "model" });
        }
        self.updates.lock().unwrap().push(update);
        Ok(())
    }

    fn logging_state(&self) -> Option<Arc<Mutex<LoggingState>>> {
        None
    }
}

pub(super) fn probe_params<T>(entries: T) -> HealthProbeParams
where
    T: serde::Serialize,
{
    let map = serde_json::from_value(serde_json::to_value(entries).unwrap()).unwrap();
    HealthProbeParams::from_map(map).unwrap()
}

pub(super) fn probe_params_from_json(text: &str) -> HealthProbeParams {
    let map = serde_json::from_str(text).unwrap();
    HealthProbeParams::from_map(map).unwrap()
}

pub(super) fn with_logging(
    mut params: HealthProbeParams,
    logging: Arc<dyn HealthProbeLogging>,
) -> HealthProbeParams {
    params.litellm_logging_obj = Some(logging);
    params
}

pub(super) async fn run_mode(
    mode: &str,
    provider: &str,
    params: HealthProbeParams,
    arguments: HealthProbeArguments,
    backend: &FakeBackend,
) -> Result<HealthProbeReply, HealthProbeError> {
    let plan = plan_health_probe(mode, "resolved-model", provider, params, arguments).unwrap();
    run_health_probe(plan, backend, &FakeDocuments { missing: false }).await
}

pub(super) fn with_prompt(prompt: Option<&str>) -> HealthProbeArguments {
    HealthProbeArguments {
        prompt: prompt.map(str::to_owned),
        input: None,
    }
}

pub(super) fn with_input(input: Option<Vec<&str>>) -> HealthProbeArguments {
    HealthProbeArguments {
        prompt: None,
        input: input.map(|values| values.into_iter().map(str::to_owned).collect()),
    }
}

const EMPTY_CONTEXT: ErrorContext = ErrorContext {
    message: String::new(),
    model: None,
    provider: None,
    max_retries: None,
    num_retries: None,
    body_type: None,
    body_param: None,
    body_code: None,
};

fn context(message: &str) -> ErrorContext {
    ErrorContext {
        message: message.to_owned(),
        ..EMPTY_CONTEXT
    }
}

pub(super) fn bad_request_class(message: &str) -> ErrorClass {
    ErrorClass::BadRequest(Box::new(ResponseDetails::from(context(message))))
}

pub(super) fn content_policy_class(message: &str) -> ErrorClass {
    ErrorClass::ContentPolicyViolation(Box::new(ResponseDetails::from(context(message))))
}

pub(super) fn context_window_class(message: &str) -> ErrorClass {
    ErrorClass::ContextWindowExceeded(Box::new(ResponseDetails::from(context(message))))
}

pub(super) fn unsupported_params_class(message: &str) -> ErrorClass {
    ErrorClass::UnsupportedParams(Box::new(context(message)))
}

pub(super) fn unclassified_class(message: &str) -> ErrorClass {
    ErrorClass::Unclassified(Box::new(context(message)))
}

pub(super) fn evaluation_params_with_questions(count: usize) -> HealthProbeParams {
    let mut entries = serde_json::json!({"model": "m", "questions": {}});
    let questions = entries["questions"].as_object_mut().unwrap();
    for index in 0..count {
        let name = format!("q{index}");
        let question = serde_json::json!({"type": "noul", "instructions": "reachable?"});
        questions.insert(name, question);
    }
    probe_params(entries)
}
