use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::soniox::pipeline::PipelineCall;
use betterlite_core::api::soniox::pipeline::SonioxPipeline;
use betterlite_core::api::soniox::pipeline::error::PipelineError;
use betterlite_core::api::soniox::pipeline::job::PipelineTarget;
use betterlite_core::api::soniox::records::response::SonioxTranscriptionResponse;
use betterlite_core::api::vector_store_http_write::service::{ClientKind, SuppliedClient};
use betterlite_openai::api::audio_file_input::{AudioFile, AudioStream, AuthorizedAudioFiles};
use betterlite_openai::openai_base_config::Object;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::input::Case;
use super::recorder::{self, Effects, Failure, Logger, NamedStream, NoFiles, Recorder};

const BOUNDARY: &str = "soniox-pipeline-boundary";
const HIDDEN_NAMES: [&str; 3] = [
    "model",
    "custom_llm_provider",
    "audio_transcription_duration",
];

static EMPTY_ENVIRONMENT: BTreeMap<String, String> = BTreeMap::new();
static NO_FILES: NoFiles = NoFiles;

#[derive(Clone, Copy)]
pub(super) enum Mode {
    Sync,
    Async,
}

pub(super) fn perform(case: &Case, mode: Mode) -> Result<Value, Failure> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(recorder::failure)?;
    let effects: Effects = Mutex::new(Vec::new());
    let outcome = run_job(&runtime, case, mode, &effects)?;
    let recorded = effects
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Ok(json!({"effects": recorded, "outcome": outcome}))
}

fn run_job(
    runtime: &Runtime,
    case: &Case,
    mode: Mode,
    effects: &Effects,
) -> Result<Value, Failure> {
    let recorder = Recorder::new(case, effects)?;
    let mut response = blank_response();
    let mut logger = Logger { effects };
    let mut stream = case
        .audio
        .as_ref()
        .map(|audio| NamedStream::new(&audio.filename, &audio.content));
    let mut audio_file = stream.as_mut().map(|held| {
        let borrowed: &mut dyn AudioStream = held;
        AudioFile::Stream(borrowed)
    });
    let call = pipeline_call(case, mode);
    let files: &(dyn AuthorizedAudioFiles + Sync) = &NO_FILES;
    let pipeline = SonioxPipeline {
        provider: &recorder,
        sleeper: &recorder,
        audio_files: files,
        limits: ApiLimits::default(),
        boundaries: || BOUNDARY.to_owned(),
    };
    let supplied = SuppliedClient {
        kind: client_kind(mode),
        transport: &recorder,
    };
    let target = PipelineTarget {
        audio_file: audio_file.as_mut(),
        response: &mut response,
        logger: &mut logger,
    };
    let outcome = runtime.block_on(pipeline.transcribe(&call, target, Some(supplied)));
    outcome_view(&response, outcome.as_ref().err())
}

fn pipeline_call(case: &Case, mode: Mode) -> PipelineCall<'_> {
    PipelineCall {
        model: &case.model,
        optional_params: &case.optional_params,
        headers: case.headers.clone().unwrap_or_default(),
        api_key: Some(&case.api_key),
        api_base: Some(&case.api_base),
        environment: &EMPTY_ENVIRONMENT,
        timeout: Duration::from_millis(case.timeout_millis),
        atranscription: matches!(mode, Mode::Async),
    }
}

const fn client_kind(mode: Mode) -> ClientKind {
    match mode {
        Mode::Sync => ClientKind::Sync,
        Mode::Async => ClientKind::Asynchronous,
    }
}

fn blank_response() -> SonioxTranscriptionResponse {
    SonioxTranscriptionResponse {
        text: Some(String::new()),
        usage: None,
        extra: Object::new(),
        hidden_params: Object::new(),
    }
}

fn outcome_view(
    response: &SonioxTranscriptionResponse,
    error: Option<&PipelineError>,
) -> Result<Value, Failure> {
    let value = serde_json::to_value(response).map_err(recorder::failure)?;
    let hidden = hidden_view(response);
    let Some(failure) = error else {
        return Ok(json!({"kind": "response", "value": value, "hidden": hidden}));
    };
    let Some(exception) = failure.provider() else {
        return Ok(json!({"kind": "failure", "value": value, "hidden": hidden}));
    };
    Ok(json!({
        "kind": "provider",
        "status": exception.status_code,
        "value": value,
        "hidden": hidden,
    }))
}

fn hidden_view(response: &SonioxTranscriptionResponse) -> Value {
    let mut present = Object::new();
    for name in HIDDEN_NAMES {
        if let Some(value) = response.hidden_params.get(name) {
            present.insert(name.to_owned(), value.clone());
        }
    }
    Value::Object(present)
}
