use std::ffi::CString;
use std::mem;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_pyhost::host::PythonHost;
use pyo3::exceptions::PyValueError;
use pyo3::types::{PyAnyMethods as _, PyModule};
use pyo3::{Bound, Py, PyAny, PyResult, Python};
use serde::Serialize;

use super::{EndpointingMode, PipelineCall, ServiceKind, StreamResponse, helper_error};

const FAKE: &str = "import json

_state = {}
_events = []
_values = {}
_chunks = []
_timeout = []
_recognition = []
_streaming = []
_streams = []

def configure(text):
    _state.clear()
    _state.update(json.loads(text))
    _events.clear()
    _values.clear()
    _chunks.clear()
    _timeout.clear()
    _recognition.clear()
    _streaming.clear()
    _streams.clear()
    _streams.extend(_state['streams'])

class _Node:
    pass

def _build(spec):
    if isinstance(spec, dict):
        node = _Node()
        for key, item in spec.items():
            setattr(node, key, _build(item))
        return node
    if isinstance(spec, list):
        return [_build(item) for item in spec]
    return spec

class Auth:
    def __init__(self, *args, **kwargs):
        if kwargs:
            _events.append('auth.kwargs')
            _values['auth.kwargs'] = json.loads(json.dumps(kwargs))
            if _state.get('legacy_auth', False):
                raise TypeError('this riva-client takes positional auth arguments')
            return
        _events.append('auth.positional')
        _values['auth.args'] = json.loads(json.dumps(list(args)))

class _CopyFrom:
    def __call__(self, source):
        _events.append('endpointing.copy')
        if not source.values:
            raise ValueError('the endpointing config carries no field')
        if _state.get('endpointing') == 'copy_fails':
            raise TypeError('cannot copy the endpointing config')
        _values['endpointing.copied'] = True

class _EndpointingTarget:
    CopyFrom = _CopyFrom()

class EndpointingConfig:
    def __init__(self, **kwargs):
        _events.append('endpointing')
        _values['endpointing'] = json.loads(json.dumps(kwargs))
        if _state.get('endpointing') == 'fails':
            raise ValueError('this riva-client knows no such endpointing field')
        self.values = kwargs

class AudioEncoding:
    LINEAR_PCM = 'LINEAR_PCM'
    FLAC = 'FLAC'
    MULAW = 'MULAW'

class RecognitionConfig:
    def __init__(self, **kwargs):
        _events.append('recognition')
        _values['recognition'] = json.loads(json.dumps(kwargs))
        _recognition.append(self)
        if _state.get('endpointing') != 'missing_field':
            self.endpointing_config = _EndpointingTarget()

class StreamingRecognitionConfig:
    def __init__(self, **kwargs):
        _events.append('streaming')
        _values['streaming.interim'] = kwargs['interim_results']
        _values['streaming.config_same'] = bool(_recognition) and kwargs['config'] is _recognition[-1]
        _streaming.append(self)
        self.config = kwargs['config']

def _generate(audio_chunks, streaming_config, timeout):
    if streaming_config is not _streaming[-1]:
        raise RuntimeError('the stream runs a streaming config that was never built')
    _events.append('stream.call')
    _chunks.append([len(chunk) for chunk in audio_chunks])
    _timeout.append(None if timeout is None else float(timeout))
    for spec in _streams:
        yield _build(spec)

class _TimeoutService:
    def streaming_response_generator(self, audio_chunks, streaming_config, timeout=None):
        return _generate(audio_chunks, streaming_config, timeout)

class _VariadicService:
    def streaming_response_generator(self, audio_chunks, **kwargs):
        return _generate(audio_chunks, kwargs['streaming_config'], kwargs['timeout'])

class _PlainService:
    def streaming_response_generator(self, audio_chunks, streaming_config):
        return _generate(audio_chunks, streaming_config, None)

_SERVICES = {'timeout': _TimeoutService, 'variadic': _VariadicService, 'plain': _PlainService}

class ASRService:
    def __new__(cls, auth):
        _events.append('asr')
        return _SERVICES[_state.get('service', 'timeout')]()

def snapshot():
    return json.dumps({
        'events': _events,
        'auth_kwargs': _values.get('auth.kwargs'),
        'auth_args': _values.get('auth.args'),
        'recognition': _values.get('recognition'),
        'endpointing_kwargs': _values.get('endpointing'),
        'endpointing_copied': _values.get('endpointing.copied', False),
        'interim_results': _values.get('streaming.interim'),
        'config_is_recognition': _values.get('streaming.config_same', False),
        'chunk_sizes': _chunks,
        'timeout_seconds': _timeout,
    })
";

// the modules each call scripts, so a fake from one call can never leak into the next
const MANAGED: [&str; 3] = ["riva", "riva.client", "soundfile"];

#[derive(Serialize)]
struct FakeConfig<'call> {
    legacy_auth: bool,
    endpointing: EndpointingMode,
    service: ServiceKind,
    streams: &'call [StreamResponse],
}

pub(super) fn install(host: &PythonHost, call: &PipelineCall) -> Result<(), Box<HelperError>> {
    host.attach(|py| install_in(py, call))
        .map_err(|error| helper_error("PythonError", error.to_string()))
}

fn install_in(py: Python<'_>, call: &PipelineCall) -> PyResult<()> {
    let modules = py.import("sys")?.getattr("modules")?;
    for name in MANAGED {
        drop(modules.del_item(name));
    }
    let client = install_module(py, "riva.client", FAKE)?;
    let config = FakeConfig {
        legacy_auth: call.legacy_auth,
        endpointing: call.endpointing,
        service: call.service,
        streams: &call.stream,
    };
    let text =
        serde_json::to_string(&config).map_err(|error| PyValueError::new_err(error.to_string()))?;
    client.call_method1("configure", (text,))?;
    let package = PyModule::new(py, "riva")?;
    package.setattr("client", &client)?;
    modules.set_item("riva", &package)?;
    modules.set_item("riva.client", &client)?;
    let soundfile = install_module(py, "soundfile", &soundfile_source(call.samples))?;
    modules.set_item("soundfile", &soundfile)?;
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

fn soundfile_source(samples: usize) -> String {
    format!(
        "import numpy as np
class _Info:
    frames = {samples}
    channels = 1
    samplerate = 16000
def info(file):
    return _Info()
def read(file, dtype='float32', always_2d=False):
    return (np.zeros({samples}, dtype=np.float32), 16000)
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
