#![cfg(test)]

use std::collections::BTreeMap;
use std::ffi::CString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use betterlite_context::non_default_params::Params;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_pyhost::host::{DEFAULT_SHUTDOWN_GRACE, HostConfig, PythonHost};
use indexmap::IndexMap;
use pyo3::types::{PyAnyMethods as _, PyModule};
use pyo3::{Bound, Python};
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::Deserialize;
use tokio::sync::Mutex;

use super::RivaCall;
use super::error::RivaCallError;
use super::logging::{RivaEvent, RivaLogger};

mod cases;
mod tracked;

// the interpreter and sys.modules are shared, so the tests run one at a time
static PYTHON: Mutex<()> = Mutex::const_new(());

const FAKE_MODULES: [&str; 7] = [
    "riva",
    "riva.client",
    "riva.client.proto",
    "riva.client.proto.riva_asr_pb2",
    "riva_call_sdk",
    "riva_call_probe",
    "soundfile",
];

const PROBE: &str = r#"import json
import threading

events = []
values = {}
flags = {}
streams = []
auth_ids = []
auth_used_ids = []
recognition_ids = []
streaming_config_ids = []
generator_config_ids = []
chunk_sizes = []
seen_languages = []
recognition_languages = {}
gate = threading.Event()


def configure(names):
    flags.clear()
    for name in names:
        flags[name] = True


def service(kind):
    flags['service'] = kind


def stage(text):
    streams[:] = json.loads(text)


def record(name, item):
    values[name] = json.dumps(item, sort_keys=True, separators=(',', ':'))


def snapshot():
    return json.dumps(
        {
            'events': events,
            'values': values,
            'auth_ids': auth_ids,
            'auth_used_ids': auth_used_ids,
            'recognition_ids': recognition_ids,
            'streaming_config_ids': streaming_config_ids,
            'generator_config_ids': generator_config_ids,
            'chunk_sizes': chunk_sizes,
            'seen_languages': seen_languages,
        },
        sort_keys=True,
        separators=(',', ':'),
    )
"#;

const SDK: &str = r#"import inspect

import riva_call_probe as probe

_ABSENT = object()
_NESTED = ('results', 'alternatives', 'words')


class _Node:
    def __init__(self, fields, raises):
        self.fields = fields
        self.raises = raises

    def __getattr__(self, name):
        if name in self.raises:
            raise RuntimeError('attribute ' + name + ' failed')
        if name in self.fields:
            return self.fields[name]
        raise AttributeError(name)


def _node(spec):
    fields = {}
    for name, item in spec.items():
        if name == 'raises':
            continue
        if name in _NESTED and isinstance(item, list):
            fields[name] = [_node(child) for child in item]
        else:
            fields[name] = item
    return _Node(fields, spec.get('raises', []))


def _generate(audio_chunks, streaming_config, timeout):
    probe.events.append('stream.call')
    probe.chunk_sizes.append([len(chunk) for chunk in audio_chunks])
    probe.record('stream.timeout', 'absent' if timeout is _ABSENT else timeout)
    key = str(id(streaming_config.config))
    if key in probe.recognition_languages:
        probe.seen_languages.append(probe.recognition_languages[key])
    else:
        probe.seen_languages.append('missing')
    probe.generator_config_ids.append(key)
    if probe.flags.get('block_stream'):
        probe.events.append('stream.blocked')
        probe.gate.wait()
    if probe.flags.get('entry_fails'):
        raise RuntimeError('stream refused')
    for spec in probe.streams:
        if 'raise' in spec:
            raise RuntimeError(spec['raise'])
        yield _node(spec)


class Auth:
    def __init__(self, *args, **kwargs):
        probe.events.append('auth')
        probe.auth_ids.append(str(id(self)))
        probe.record('auth.kwargs', kwargs)
        probe.record('auth.args', list(args))
        if probe.flags.get('auth_legacy') and kwargs:
            raise TypeError('legacy signature is positional')
        if probe.flags.get('auth_legacy'):
            return
        if probe.flags.get('auth_type_error'):
            raise TypeError('broken auth')
        if probe.flags.get('auth_fails'):
            raise RuntimeError('auth refused')


class _EndpointingTarget:
    def CopyFrom(self, source):
        probe.events.append('endpointing.copy')
        probe.record('endpointing.copied', source.kwargs)
        if probe.flags.get('endpointing_copy_fails'):
            raise TypeError('cannot copy')


class EndpointingConfig:
    def __init__(self, **kwargs):
        probe.events.append('endpointing')
        probe.record('endpointing.kwargs', kwargs)
        if probe.flags.get('endpointing_init_fails'):
            raise ValueError('unknown endpointing field')
        self.kwargs = kwargs


class AudioEncoding:
    LINEAR_PCM = 1
    FLAC = 2
    MULAW = 3


class RecognitionConfig:
    def __init__(self, **kwargs):
        probe.events.append('recognition')
        probe.recognition_ids.append(str(id(self)))
        probe.record('recognition.kwargs', kwargs)
        probe.recognition_languages[str(id(self))] = kwargs.get('language_code')
        if probe.flags.get('recognition_fails'):
            raise RuntimeError('recognition refused')
        if not probe.flags.get('endpointing_field_missing'):
            self.endpointing_config = _EndpointingTarget()


class StreamingRecognitionConfig:
    def __init__(self, **kwargs):
        probe.events.append('streaming')
        probe.record('streaming.interim_results', kwargs.get('interim_results'))
        probe.record('streaming.keys', sorted(kwargs))
        self.config = kwargs.get('config')
        probe.streaming_config_ids.append(str(id(self)))


class _TimeoutService:
    def streaming_response_generator(self, audio_chunks, streaming_config, timeout=None):
        return _generate(audio_chunks, streaming_config, timeout)


class _VariadicService:
    def streaming_response_generator(self, audio_chunks, streaming_config, **kwargs):
        probe.record('stream.extra', sorted(kwargs))
        timeout = kwargs.get('timeout', _ABSENT)
        return _generate(audio_chunks, streaming_config, timeout)


class _PlainService:
    def streaming_response_generator(self, audio_chunks, streaming_config):
        return _generate(audio_chunks, streaming_config, _ABSENT)


class _BrokenService:
    def streaming_response_generator(self, audio_chunks, streaming_config):
        return _generate(audio_chunks, streaming_config, _ABSENT)


_BrokenService.streaming_response_generator.__signature__ = 5


class _RaisingService:
    def streaming_response_generator(self, audio_chunks, streaming_config, **kwargs):
        raise RuntimeError('generator refused')


class _ScalarService:
    def streaming_response_generator(self, audio_chunks, streaming_config, **kwargs):
        return 5


_SERVICES = {
    'timeout': _TimeoutService,
    'variadic': _VariadicService,
    'plain': _PlainService,
    'broken': _BrokenService,
    'raises': _RaisingService,
    'scalar': _ScalarService,
}


class _Code:
    name = 'UNAVAILABLE'


class GrpcError(Exception):
    def code(self):
        return _Code()

    def details(self):
        return 'backend down'


class ASRService:
    def __new__(cls, auth):
        probe.events.append('asr')
        probe.auth_used_ids.append(str(id(auth)))
        if probe.flags.get('grpc_failure'):
            raise GrpcError('unavailable')
        if probe.flags.get('asr_fails'):
            raise RuntimeError('asr refused')
        return object.__new__(_SERVICES[probe.flags.get('service', 'timeout')])
"#;

const CLIENT_FULL: &str = r#"from riva_call_sdk import (
    ASRService,
    AudioEncoding,
    Auth,
    EndpointingConfig,
    RecognitionConfig,
    StreamingRecognitionConfig,
)
"#;

const CLIENT_WITHOUT_CONFIGS: &str = r#"from riva_call_sdk import ASRService, Auth
"#;

const CLIENT_BROKEN: &str = r#"def __getattr__(name):
    raise RuntimeError('client attribute failed')
"#;

const PROTO: &str = r#"from riva_call_sdk import AudioEncoding, EndpointingConfig, RecognitionConfig, StreamingRecognitionConfig
"#;

// what the probe and the fake SDK recorded during one test
#[derive(Debug, Default, Deserialize)]
struct Probe {
    #[serde(default)]
    events: Vec<String>,
    #[serde(default)]
    values: BTreeMap<String, String>,
    #[serde(default)]
    auth_ids: Vec<String>,
    #[serde(default)]
    auth_used_ids: Vec<String>,
    #[serde(default)]
    recognition_ids: Vec<String>,
    #[serde(default)]
    streaming_config_ids: Vec<String>,
    #[serde(default)]
    generator_config_ids: Vec<String>,
    #[serde(default)]
    chunk_sizes: Vec<Vec<usize>>,
    #[serde(default)]
    seen_languages: Vec<String>,
}

// the arguments the logger would hand to a LiteLLM callback
#[derive(Default)]
struct RecordingLogger {
    pre: Vec<LoggedPre>,
    post: Vec<LoggedPost>,
}

struct LoggedPre {
    api_base: String,
    atranscription: bool,
    nvcf_function_id_set: bool,
    use_ssl: Json,
    recognition_config: IndexMap<String, Json>,
}

struct LoggedPost {
    input: Json,
    original_response: Json,
    recognition_config: IndexMap<String, Json>,
}

impl RivaLogger for RecordingLogger {
    fn record(&mut self, event: RivaEvent) -> Result<(), PrePostError> {
        match event {
            RivaEvent::PreCall(record) => {
                self.pre.push(LoggedPre {
                    api_base: record.api_base,
                    atranscription: record.atranscription,
                    nvcf_function_id_set: record.nvcf_function_id_set,
                    use_ssl: record.use_ssl,
                    recognition_config: record.recognition_config,
                });
            }
            RivaEvent::PostCall(record) => {
                self.post.push(LoggedPost {
                    input: record.input,
                    original_response: record.original_response,
                    recognition_config: record.recognition_config,
                });
            }
        }
        Ok(())
    }
}

// the handler's arguments after B047 resolved the endpoint and the key
struct Setup {
    model: String,
    input: String,
    file: ProcessedAudioFile,
    params: Params,
    key: Option<SecretString>,
}

impl Setup {
    fn new(optional: &str) -> Self {
        let params: Params = match serde_json::from_str(optional) {
            Ok(parsed) => parsed,
            Err(error) => panic!("the test params are invalid: {error}"),
        };
        Self {
            model: String::from("nvidia-riva/parakeet"),
            input: String::from("clip.wav"),
            file: ProcessedAudioFile {
                file_content: b"fake audio".to_vec(),
                filename: String::from("clip.wav"),
                content_type: String::from("audio/wav"),
            },
            params,
            key: Some(SecretString::from("riva-key".to_owned())),
        }
    }

    fn without_key(mut self) -> Self {
        self.key = None;
        self
    }
}

fn call<'setup>(
    setup: &'setup Setup,
    api_base: Option<&'setup str>,
    atranscription: bool,
) -> RivaCall<'setup> {
    RivaCall {
        model: &setup.model,
        input: &setup.input,
        file: &setup.file,
        optional_params: &setup.params,
        atranscription,
        timeout: Duration::from_secs(30),
        api_base,
        api_key: setup.key.as_ref(),
    }
}

fn site_packages() -> Option<PathBuf> {
    let lib = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/oracle/litellm/.venv/lib");
    fs::read_dir(lib)
        .ok()?
        .flatten()
        .map(|entry| entry.path().join("site-packages"))
        .find(|folder| folder.is_dir())
}

fn audio_host() -> Option<PythonHost> {
    let host = PythonHost::start(&HostConfig {
        module_dirs: vec![site_packages()?],
        shutdown_grace: DEFAULT_SHUTDOWN_GRACE,
    })
    .ok()?;
    host.import_check("numpy").ok()?;
    Some(host)
}

fn modules(py: Python<'_>) -> Bound<'_, PyAny> {
    py.import("sys").unwrap().getattr("modules").unwrap()
}

fn install<'py>(py: Python<'py>, name: &str, source: &str) -> Bound<'py, PyModule> {
    let code = CString::new(source).unwrap();
    let file_name = CString::new(format!("{name}.py")).unwrap();
    let module_name = CString::new(name).unwrap();
    PyModule::from_code(py, &code, &file_name, &module_name).unwrap()
}

fn install_fakes(host: &PythonHost, samples: usize, client_source: &str, proto: bool) {
    host.attach(|py| {
        let registered = modules(py);
        for name in FAKE_MODULES {
            drop(registered.del_item(name));
        }
        let probe = install(py, "riva_call_probe", PROBE);
        registered.set_item("riva_call_probe", &probe).unwrap();
        let sdk = install(py, "riva_call_sdk", SDK);
        registered.set_item("riva_call_sdk", &sdk).unwrap();
        let soundfile = install(py, "soundfile", &soundfile_source(samples));
        registered.set_item("soundfile", &soundfile).unwrap();
        let client = install(py, "riva.client", client_source);
        let riva = PyModule::new(py, "riva").unwrap();
        riva.setattr("client", &client).unwrap();
        registered.set_item("riva", &riva).unwrap();
        registered.set_item("riva.client", &client).unwrap();
        if proto {
            let proto_package = PyModule::new(py, "riva.client.proto").unwrap();
            let asr_pb2 = install(py, PROTO_NAME, PROTO);
            proto_package.setattr("riva_asr_pb2", &asr_pb2).unwrap();
            client.setattr("proto", &proto_package).unwrap();
            registered
                .set_item("riva.client.proto", &proto_package)
                .unwrap();
            registered.set_item(PROTO_NAME, &asr_pb2).unwrap();
        }
    });
}

const PROTO_NAME: &str = "riva.client.proto.riva_asr_pb2";

fn reset(host: &PythonHost, samples: usize) {
    install_fakes(host, samples, CLIENT_FULL, true);
}

fn soundfile_source(samples: usize) -> String {
    let frames = format!("    frames = {samples}\n    channels = 1\n    samplerate = 16000\n");
    let samples_block = format!("{samples}");
    format!(
        "import numpy as np\nclass _Info:\n{frames}def info(file):\n    return _Info()\ndef read(file, dtype='float32', always_2d=False):\n    return (np.zeros({samples_block}, dtype=np.float32), 16000)\n"
    )
}

fn probe(host: &PythonHost) -> Probe {
    let text: String = host.attach(|py| {
        let module = py.import("riva_call_probe").unwrap();
        let snapshot = module.call_method0("snapshot").unwrap();
        snapshot.extract().unwrap()
    });
    let parsed: Probe = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(error) => panic!("the probe snapshot is invalid: {error}"),
    };
    parsed
}

fn value(probe: &Probe, name: &str) -> String {
    match probe.values.get(name) {
        Some(value) => value.clone(),
        None => panic!("the probe never recorded {name}"),
    }
}

fn configure(host: &PythonHost, flags: &[&str]) {
    host.attach(|py| {
        let module = py.import("riva_call_probe").unwrap();
        let list: Vec<String> = flags.iter().map(|flag| (*flag).to_owned()).collect();
        module.call_method1("configure", (list,)).unwrap();
    });
}

fn service(host: &PythonHost, kind: &str) {
    host.attach(|py| {
        let module = py.import("riva_call_probe").unwrap();
        module.call_method1("service", (kind,)).unwrap();
    });
}

fn stage(host: &PythonHost, specs: &str) {
    host.attach(|py| {
        let module = py.import("riva_call_probe").unwrap();
        module.call_method1("stage", (specs,)).unwrap();
    });
}

fn json(source: &str) -> Json {
    match serde_json::from_str(source) {
        Ok(value) => value,
        Err(error) => panic!("the test JSON is invalid: {error}"),
    }
}

fn decimal(source: &str) -> Decimal {
    match source.parse() {
        Ok(value) => value,
        Err(error) => panic!("the test decimal is invalid: {error}"),
    }
}
