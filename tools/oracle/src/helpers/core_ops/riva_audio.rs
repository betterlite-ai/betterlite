mod answer;
mod fakes;

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_core::api::riva::audio::decode_riva_audio_attached;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_oracle_runner::helpers::ORACLE_DIR;
use betterlite_pyhost::host::{DEFAULT_SHUTDOWN_GRACE, HostConfig, PythonHost};
use serde::Deserialize;
use serde_json::{Value, json};

use self::answer::{answer, failure_answer, read_probe};
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
    file_hex: String,
    soundfile_error: Option<String>,
    audioread: Option<AudioreadSpec>,
    resamplers: Vec<Resampler>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioreadSpec {
    chunks: Vec<String>,
    samplerate: i64,
    channels: i64,
    failure: Failure,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Failure {
    None,
    Open,
    Iteration,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Resampler {
    Soxr,
    Scipy,
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
    let _guard = SavedModules::capture(host);
    install(host, call)?;
    let outcome = host.attach(|py| decode_riva_audio_attached(py, &file_bytes));
    let probe = read_probe(host)?;
    Ok(match outcome {
        Ok(audio) => answer(probe, Value::Null, Value::Null, Some(&audio)),
        Err(error) => failure_answer(&error, probe),
    })
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
