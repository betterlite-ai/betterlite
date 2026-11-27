use std::ffi::{CString, OsStr};
use std::fs;
use std::mem;
use std::path::{Path, PathBuf};

use betterlite_core::api::riva::audio::decode_riva_audio_blocking;
use betterlite_core::api::riva::audio::error::RivaAudioError;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_oracle_runner::helpers::ORACLE_DIR;
use betterlite_pyhost::host::{DEFAULT_SHUTDOWN_GRACE, HostConfig, PythonHost};
use pyo3::exceptions::PyValueError;
use pyo3::types::{PyAnyMethods as _, PyModule};
use pyo3::{Bound, Py, PyAny, PyResult, Python};
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

const PROBE: &str = "events = []\nstaging = []\n";

const SOXR: &str = "import riva_audio_probe as probe
def resample(samples, source_rate, target_rate):
    probe.events.append('soxr.resample:' + str(source_rate) + ':' + str(target_rate))
    return samples
";

const SCIPY_SIGNAL: &str = "import riva_audio_probe as probe
def resample_poly(samples, up, down):
    probe.events.append('scipy.resample_poly:' + str(up) + ':' + str(down))
    return samples
";

// the modules each call scripts, so a fake from one call can never leak into the next
const MANAGED: [&str; 7] = [
    "riva_audio_probe",
    "soundfile",
    "audioread",
    "soxr",
    "scipy",
    "scipy.signal",
    "scipy_signal",
];

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

struct Probe {
    events: Vec<String>,
    staging: Vec<String>,
}

struct StagingState<'staging> {
    suffix: Option<&'staging str>,
    removed: Option<bool>,
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
            format!("the oracle venv is not synced under {lib:?}"),
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
        format!("the oracle venv holds no site-packages under {lib:?}"),
    ))
}

fn run_call(host: &PythonHost, call: &PipelineCall) -> Result<Value, Box<HelperError>> {
    let file_bytes = hex::decode(&call.file_hex)
        .map_err(|error| helper_error("InvalidInput", error.to_string()))?;
    let _guard = SavedModules::capture(host);
    install(host, call)?;
    let outcome = decode_riva_audio_blocking(host, &file_bytes);
    let probe = read_probe(host)?;
    Ok(match outcome {
        Ok(audio) => success_answer(&audio, probe),
        Err(error) => failure_answer(&error, probe),
    })
}

fn install(host: &PythonHost, call: &PipelineCall) -> Result<(), Box<HelperError>> {
    host.attach(|py| install_in(py, call))
        .map_err(|error| helper_error("PythonError", error.to_string()))
}

fn install_in(py: Python<'_>, call: &PipelineCall) -> PyResult<()> {
    let modules = python_modules(py)?;
    for name in MANAGED {
        drop(modules.del_item(name));
    }
    drop(install_module(py, "riva_audio_probe", PROBE)?);
    if let Some(message) = &call.soundfile_error {
        drop(install_module(py, "soundfile", &soundfile_source(message))?);
    }
    match &call.audioread {
        Some(spec) => drop(install_module(py, "audioread", &audioread_source(spec))?),
        None => modules.set_item("audioread", py.None())?,
    }
    if call.resamplers.contains(&Resampler::Soxr) {
        drop(install_module(py, "soxr", SOXR)?);
    } else {
        modules.set_item("soxr", py.None())?;
    }
    if call.resamplers.contains(&Resampler::Scipy) {
        let signal = install_module(py, "scipy_signal", SCIPY_SIGNAL)?;
        let scipy = PyModule::new(py, "scipy")?;
        scipy.setattr("signal", &signal)?;
        modules.set_item("scipy", &scipy)?;
        modules.set_item("scipy.signal", &signal)?;
    } else {
        modules.set_item("scipy", py.None())?;
        modules.set_item("scipy.signal", py.None())?;
    }
    Ok(())
}

fn install_module<'py>(
    py: Python<'py>,
    name: &str,
    source: &str,
) -> PyResult<Bound<'py, PyModule>> {
    let code = CString::new(source)
        .map_err(|_nul| PyValueError::new_err("the fake module source holds a NUL byte"))?;
    let file_name = CString::new(format!("{name}.py"))
        .map_err(|_nul| PyValueError::new_err("the fake module name holds a NUL byte"))?;
    let module_name = CString::new(name)
        .map_err(|_nul| PyValueError::new_err("the fake module name holds a NUL byte"))?;
    PyModule::from_code(py, &code, &file_name, &module_name)
}

fn read_probe(host: &PythonHost) -> Result<Probe, Box<HelperError>> {
    host.attach(|py| {
        let module = py.import("riva_audio_probe")?;
        let events: Vec<String> = module.getattr("events")?.extract()?;
        let staging: Vec<String> = module.getattr("staging")?.extract()?;
        Ok(Probe { events, staging })
    })
    .map_err(|error| helper_error("PythonError", error.to_string()))
}

fn success_answer(audio: &ResampledAudio, probe: Probe) -> Value {
    let Probe { events, staging } = probe;
    let state = staging_state(&staging);
    json!({
        "status": Value::Null,
        "message": Value::Null,
        "pcm_hex": hex::encode(audio.pcm_bytes.as_slice()),
        "sample_rate_hz": audio.sample_rate_hz,
        "num_channels": audio.num_channels,
        "duration_seconds": audio.duration_seconds.to_f64(),
        "events": events,
        "staging_suffix": state.suffix,
        "staging_removed": state.removed,
    })
}

fn failure_answer(error: &RivaAudioError, probe: Probe) -> Value {
    let Probe { events, staging } = probe;
    let state = staging_state(&staging);
    json!({
        "status": error.status_code(),
        "message": error.to_string(),
        "pcm_hex": Value::Null,
        "sample_rate_hz": Value::Null,
        "num_channels": Value::Null,
        "duration_seconds": Value::Null,
        "events": events,
        "staging_suffix": state.suffix,
        "staging_removed": state.removed,
    })
}

fn staging_state(staging: &[String]) -> StagingState<'_> {
    let Some(path) = staging.last() else {
        return StagingState {
            suffix: None,
            removed: None,
        };
    };
    let suffix = Path::new(path).extension().and_then(OsStr::to_str);
    StagingState {
        suffix,
        removed: Some(!Path::new(path).exists()),
    }
}

fn python_modules(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("sys")?.getattr("modules")
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

fn soundfile_source(message: &str) -> String {
    format!(
        "class _Error(Exception):
    pass
def info(file):
    raise _Error({message:?})
def read(file, dtype='float32', always_2d=False):
    raise _Error({message:?})
"
    )
}

fn audioread_source(spec: &AudioreadSpec) -> String {
    let chunks: Vec<String> = spec
        .chunks
        .iter()
        .map(|chunk| format!("bytes.fromhex({chunk:?})"))
        .collect();
    let joined = chunks.join(", ");
    let samplerate = spec.samplerate;
    let channels = spec.channels;
    let iteration = if matches!(spec.failure, Failure::Iteration) {
        "True"
    } else {
        "False"
    };
    let open = if matches!(spec.failure, Failure::Open) {
        "True"
    } else {
        "False"
    };
    format!(
        "import riva_audio_probe as probe
class _Source:
    samplerate = {samplerate}
    channels = {channels}
    def __enter__(self):
        probe.events.append('audioread.enter')
        return self
    def __exit__(self, *args):
        probe.events.append('audioread.exit')
    def __iter__(self):
        if {iteration}:
            raise RuntimeError('cannot read audio')
        for chunk in [{joined}]:
            probe.events.append('audioread.read')
            yield chunk
class _Open:
    def __call__(self, path):
        probe.staging.append(path)
        probe.events.append('audioread.open')
        if {open}:
            raise RuntimeError('cannot open audio')
        return _Source()
audio_open = _Open()
"
    )
}

// every managed module comes back after the run, so a fake never reaches another oracle case
struct SavedModules<'host> {
    host: &'host PythonHost,
    saved: Vec<(&'static str, Option<Py<PyAny>>)>,
}

impl<'host> SavedModules<'host> {
    fn capture(host: &'host PythonHost) -> Self {
        let saved = host.attach(|py| {
            let mut saved = Vec::with_capacity(MANAGED.len());
            let Ok(modules) = python_modules(py) else {
                return saved;
            };
            for name in MANAGED {
                let previous = modules.get_item(name).ok().map(Bound::unbind);
                saved.push((name, previous));
            }
            saved
        });
        Self { host, saved }
    }
}

impl Drop for SavedModules<'_> {
    fn drop(&mut self) {
        let saved = mem::take(&mut self.saved);
        self.host.attach(move |py| {
            let Ok(modules) = python_modules(py) else {
                return;
            };
            for (name, previous) in saved {
                match previous {
                    Some(module) => drop(modules.set_item(name, module.bind(py))),
                    None => drop(modules.del_item(name)),
                }
            }
        });
    }
}
