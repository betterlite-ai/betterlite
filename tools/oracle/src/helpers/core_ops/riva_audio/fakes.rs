use std::ffi::CString;
use std::mem;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_pyhost::host::PythonHost;
use pyo3::exceptions::PyValueError;
use pyo3::types::{PyAnyMethods as _, PyModule};
use pyo3::{Bound, Py, PyAny, PyResult, Python};

use super::{AudioreadSpec, Failure, PipelineCall, Resampler, helper_error};

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

pub(super) fn install(host: &PythonHost, call: &PipelineCall) -> Result<(), Box<HelperError>> {
    host.attach(|py| install_in(py, call))
        .map_err(|error| helper_error("PythonError", error.to_string()))
}

fn install_in(py: Python<'_>, call: &PipelineCall) -> PyResult<()> {
    let modules = py.import("sys")?.getattr("modules")?;
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
pub(super) struct SavedModules<'host> {
    host: &'host PythonHost,
    saved: Vec<(&'static str, Option<Py<PyAny>>)>,
}

impl<'host> SavedModules<'host> {
    pub(super) fn capture(host: &'host PythonHost) -> Self {
        let saved = host.attach(|py| {
            let mut saved = Vec::with_capacity(MANAGED.len());
            let Ok(modules) = py.import("sys").and_then(|sys| sys.getattr("modules")) else {
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
            let Ok(modules) = py.import("sys").and_then(|sys| sys.getattr("modules")) else {
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
