use async_trait::async_trait;
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_logging::logging_state::{LoggingState, LoggingStateError};
use betterlite_models::decisions::Json;
use betterlite_ocr::ocr_document::OcrDocument;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::health_probe_modes::backend::{
    CheapestChatModels, HealthProbeBackend, OcrHealthDocuments,
};
use betterlite_server::health_probe_modes::calls::{
    AudioTranscriptionCall, EvaluationCall, HealthProbeReply, ImageEditCall, OcrCall, ProbeCall,
    RealtimeCall,
};
use betterlite_server::health_probe_modes::error::HealthProbeError;
use betterlite_server::health_probe_modes::logging::{HealthProbeLogging, ProbeLoggingUpdate};
use betterlite_server::health_probe_modes::params::{HealthProbeParams, Param};
use betterlite_server::health_probe_modes::plan::{HealthProbeArguments, plan_health_probe};
use betterlite_server::health_probe_modes::response::health_check_response;
use betterlite_server::health_probe_modes::run::run_health_probe;
use betterlite_server::health_probe_modes::tracking::add_tracking_information;
use betterlite_server::health_probe_modes::wildcard::probe_wildcard_model;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::runtime::Builder;
const TRACKED: [&str; 7] = [
    "tags",
    "user_api_key",
    "user_api_key_hash",
    "user_api_key_alias",
    "user_api_key_team_id",
    "user_api_key_team_alias",
    "user_api_key_spend",
];
#[derive(Deserialize)]
struct Input {
    mode: Vec<ModeStep>,
    wildcard: Vec<WildcardStep>,
    tracking: Vec<TrackingStep>,
    response: Vec<ResponseStep>,
}
#[derive(Clone, Deserialize)]
struct Failure {
    class_name: String,
    message: String,
}
#[derive(Deserialize)]
struct ModeStep {
    mode: String,
    model: String,
    custom_llm_provider: String,
    params: serde_json::Map<String, Value>,
    prompt: Option<String>,
    inputs: Option<Vec<String>>,
    logging: bool,
    failure: Option<Failure>,
}
#[derive(Deserialize)]
struct WildcardStep {
    model: String,
    custom_llm_provider: String,
    params: serde_json::Map<String, Value>,
    cheapest: Vec<String>,
}
#[derive(Deserialize)]
struct TrackingStep {
    params: serde_json::Map<String, Value>,
}
#[derive(Deserialize)]
struct ResponseStep {
    headers: BTreeMap<String, String>,
}
struct Recorder {
    calls: Mutex<Vec<Value>>,
    failure: Option<Failure>,
}
impl Recorder {
    fn new(failure: Option<&Failure>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failure: failure.cloned(),
        }
    }
    fn first(&self) -> Option<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
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
#[async_trait]
impl HealthProbeBackend for Recorder {
    async fn chat(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("acompletion", arguments_of(&call.parameters))
    }
    async fn completion(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("atext_completion", arguments_of(&call.parameters))
    }
    async fn embedding(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("aembedding", arguments_of(&call.parameters))
    }
    async fn audio_speech(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("aspeech", arguments_of(&call.parameters))
    }
    async fn audio_transcription(
        &self,
        call: AudioTranscriptionCall,
    ) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("atranscription", arguments_of(&call.parameters))
    }
    async fn image_generation(
        &self,
        call: ProbeCall,
    ) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("aimage_generation", arguments_of(&call.parameters))
    }
    async fn image_edit(&self, call: ImageEditCall) -> Result<HealthProbeReply, HealthProbeError> {
        let mut arguments = arguments_of(&call.parameters);
        if let Value::Object(entries) = &mut arguments {
            entries.insert("prompt".to_owned(), Value::String(call.prompt.to_owned()));
        }
        self.note("aimage_edit", arguments)
    }
    async fn video_generation(
        &self,
        call: ProbeCall,
    ) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("avideo_generation", arguments_of(&call.parameters))
    }
    async fn rerank(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("arerank", arguments_of(&call.parameters))
    }
    async fn realtime(&self, call: RealtimeCall) -> Result<HealthProbeReply, HealthProbeError> {
        let arguments = json!({
            "model": call.model,
            "custom_llm_provider": call.custom_llm_provider,
            "api_base": call.api_base,
            "api_version": call.api_version,
            "model_params": arguments_of(&call.model_params),
        });
        self.note("realtime_health_check", arguments)
    }
    async fn batch(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("alist_batches", arguments_of(&call.parameters))
    }
    async fn responses(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("aresponses", arguments_of(&call.parameters))
    }
    async fn ocr(&self, call: OcrCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("aocr", arguments_of(&call.parameters))
    }
    async fn evaluation(&self, call: EvaluationCall) -> Result<HealthProbeReply, HealthProbeError> {
        self.note("asystemone", arguments_of(&call.params))
    }
}
#[derive(Default)]
struct UpdateRecorder {
    update: Mutex<Option<Value>>,
}
impl UpdateRecorder {
    fn recorded(&self) -> Option<Value> {
        self.update
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
impl HealthProbeLogging for UpdateRecorder {
    fn update_from_kwargs(&self, update: ProbeLoggingUpdate) -> Result<(), LoggingStateError> {
        let kwargs = arguments_of(&update.kwargs);
        let optional_params = arguments_of(&update.optional_params);
        let litellm_params = update.litellm_params.as_ref().map(arguments_of);
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
struct FixedCheapest {
    models: Vec<String>,
}
impl CheapestChatModels for FixedCheapest {
    fn cheapest_chat_models(&self, _custom_llm_provider: &str, n: usize) -> Vec<String> {
        self.models.iter().take(n).cloned().collect()
    }
}
struct MissingDocuments;
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
fn arguments_of<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}
fn parameters(
    given: &serde_json::Map<String, Value>,
) -> Result<HealthProbeParams, HealthProbeError> {
    let entries: IndexMap<String, Value> = given.clone().into_iter().collect();
    HealthProbeParams::from_map(entries)
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
fn error_value(kind: &str, detail: &str) -> Value {
    json!({"error": {"kind": kind, "detail": detail}})
}
fn failure_tag(step: &ModeStep) -> &str {
    match &step.failure {
        Some(failure) => failure.class_name.as_str(),
        None => "",
    }
}
fn mode_error(step: &ModeStep, error: &HealthProbeError) -> Value {
    match error {
        HealthProbeError::RepeatedParameter { key, .. } => error_value("repeated", key),
        HealthProbeError::EvaluationPayload { field, .. } => error_value("payload", field),
        HealthProbeError::Backend { .. } => error_value("backend", failure_tag(step)),
        HealthProbeError::ParameterType { .. }
        | HealthProbeError::NoCheapModel { .. }
        | HealthProbeError::OcrDocumentUnavailable { .. }
        | HealthProbeError::HealthImage { .. }
        | HealthProbeError::Logging { .. }
        | HealthProbeError::Principal { .. }
        | HealthProbeError::PrincipalEncode { .. }
        | HealthProbeError::Tracking { .. }
        | HealthProbeError::TrackingMetadata => error_value("other", &error.to_string()),
    }
}
fn success_outcome(step: &ModeStep, calls: &Recorder, updates: &UpdateRecorder) -> Value {
    let result = if step.failure.is_some() {
        "empty"
    } else {
        "ok"
    };
    json!({
        "call": calls.first(),
        "logging": updates.recorded(),
        "result": result,
    })
}
async fn mode_step(step: &ModeStep) -> Value {
    let mut params = match parameters(&step.params) {
        Ok(params) => params,
        Err(error) => return error_value("other", &error.to_string()),
    };
    let updates = Arc::new(UpdateRecorder::default());
    if step.logging {
        let logging: Arc<dyn HealthProbeLogging> = updates.clone();
        params.litellm_logging_obj = Some(logging);
    }
    let arguments = HealthProbeArguments {
        prompt: step.prompt.clone(),
        input: step.inputs.clone(),
    };
    let Some(plan) = plan_health_probe(
        &step.mode,
        &step.model,
        &step.custom_llm_provider,
        params,
        arguments,
    ) else {
        return error_value("mode", &step.mode);
    };
    let calls = Recorder::new(step.failure.as_ref());
    match run_health_probe(plan, &calls, &MissingDocuments).await {
        Ok(_) => success_outcome(step, &calls, &updates),
        Err(error) => mode_error(step, &error),
    }
}
async fn wildcard_step(step: &WildcardStep) -> Value {
    let params = match parameters(&step.params) {
        Ok(params) => params,
        Err(error) => return error_value("other", &error.to_string()),
    };
    let calls = Recorder::new(None);
    let cheapest = FixedCheapest {
        models: step.cheapest.clone(),
    };
    match probe_wildcard_model(
        &step.model,
        &step.custom_llm_provider,
        params,
        &cheapest,
        &calls,
    )
    .await
    {
        Ok(_) => json!({"call": calls.first()}),
        Err(error) => error_value("no_model", &error.to_string()),
    }
}
fn tracking_step(step: &TrackingStep) -> Value {
    let params = match parameters(&step.params) {
        Ok(params) => params,
        Err(error) => return error_value("other", &error.to_string()),
    };
    let tracked = match add_tracking_information(params) {
        Ok(tracked) => tracked,
        Err(error) => return error_value("other", &error.to_string()),
    };
    let Param::Value(Json::Object(metadata)) = tracked.litellm_metadata else {
        return error_value("other", "the tracking metadata is not an object");
    };
    let mut entries = serde_json::Map::new();
    for key in TRACKED {
        let value = metadata.get(key).map_or(Value::Null, arguments_of);
        entries.insert(key.to_owned(), value);
    }
    Value::Object(entries)
}
fn response_step(step: &ResponseStep) -> Value {
    arguments_of(&health_check_response(&step.headers))
}
async fn run_steps(input: &Input) -> Vec<Value> {
    let mut results = Vec::new();
    for step in &input.mode {
        results.push(mode_step(step).await);
    }
    for step in &input.wildcard {
        results.push(wildcard_step(step).await);
    }
    for step in &input.tracking {
        results.push(tracking_step(step));
    }
    for step in &input.response {
        results.push(response_step(step));
    }
    results
}
pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().enable_all().build()?;
        let results = runtime.block_on(run_steps(input));
        Ok(Value::Array(results))
    }
}
