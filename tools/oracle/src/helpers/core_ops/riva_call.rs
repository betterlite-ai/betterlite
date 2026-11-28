mod answer;
mod fakes;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use betterlite_context::non_default_params::Params;
use betterlite_core::api::riva::call::{RivaCall, transcribe_attached};
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_oracle_runner::helpers::{ORACLE_DIR, sync_litellm};
use betterlite_pyhost::host::{DEFAULT_SHUTDOWN_GRACE, HostConfig, PythonHost};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use self::answer::{RecordingLogger, answer, failure_answer, read_probe};
use self::fakes::{SavedModules, install};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<PipelineCall>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PipelineCall {
    model: String,
    audio_name: String,
    file_hex: String,
    samples: usize,
    optional_params: Params,
    api_base: Option<String>,
    api_key: Option<String>,
    timeout_seconds: f64,
    service: ServiceKind,
    legacy_auth: bool,
    endpointing: EndpointingMode,
    stream: Vec<StreamResponse>,
}

#[derive(Deserialize, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ServiceKind {
    Timeout,
    Variadic,
    Plain,
}

#[derive(Deserialize, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum EndpointingMode {
    Ok,
    Fails,
    CopyFails,
    MissingField,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StreamResponse {
    results: Vec<StreamResult>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StreamResult {
    is_final: bool,
    alternatives: Vec<StreamAlternative>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StreamAlternative {
    transcript: String,
    words: Vec<StreamWord>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StreamWord {
    word: String,
    start_time: i64,
    end_time: i64,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let host = host()?;
        let mut answers = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            answers.push(run_call(&host, call)?);
        }
        Ok(json!(answers))
    }
}

fn host() -> Result<PythonHost, Box<HelperError>> {
    PythonHost::start(&HostConfig {
        module_dirs: vec![site_packages()?],
        shutdown_grace: DEFAULT_SHUTDOWN_GRACE,
    })
    .map_err(|error| helper_error("PyHostError", error.to_string()))
}

fn site_packages() -> Result<PathBuf, Box<HelperError>> {
    let lib = Path::new(ORACLE_DIR).join("litellm/.venv/lib");
    if !lib.is_dir() {
        // the answer side syncs the same pinned venv, and the first native case runs before it does
        sync_litellm(&Path::new(ORACLE_DIR).join("../.."))
            .map_err(|error| helper_error("PyHostError", error.to_string()))?;
    }
    let Ok(entries) = fs::read_dir(&lib) else {
        return Err(helper_error(
            "PyHostError",
            format!("the oracle venv is not synced under {}", lib.display()),
        ));
    };
    for entry in entries.flatten() {
        let folder = entry.path().join("site-packages");
        if folder.is_dir() {
            return Ok(folder);
        }
    }
    Err(helper_error(
        "PyHostError",
        format!(
            "the oracle venv holds no site-packages under {}",
            lib.display()
        ),
    ))
}

fn run_call(host: &PythonHost, call: &PipelineCall) -> Result<Value, Box<HelperError>> {
    let file_bytes = hex::decode(&call.file_hex)
        .map_err(|error| helper_error("InvalidInput", error.to_string()))?;
    let file = ProcessedAudioFile {
        file_content: file_bytes,
        filename: call.audio_name.clone(),
        content_type: String::from("audio/wav"),
    };
    let _guard = SavedModules::capture(host);
    install(host, call)?;
    let timeout = Duration::try_from_secs_f64(call.timeout_seconds)
        .map_err(|error| helper_error("InvalidInput", error.to_string()))?;
    let key = call
        .api_key
        .as_ref()
        .map(|text| SecretString::from(text.clone()));
    let pending = RivaCall {
        model: &call.model,
        input: &call.audio_name,
        file: &file,
        optional_params: &call.optional_params,
        atranscription: false,
        timeout,
        api_base: call.api_base.as_deref(),
        api_key: key.as_ref(),
    };
    let mut logger = RecordingLogger::default();
    let outcome = host.attach(|py| transcribe_attached(py, &pending, &mut logger));
    let probe = read_probe(host)?;
    match outcome {
        Ok(reply) => answer(&probe, &logger, &reply),
        Err(error) => failure_answer(&probe, &logger, &error),
    }
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
