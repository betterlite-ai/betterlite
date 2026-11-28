mod config;

use betterlite_models::decisions::Json;
use betterlite_pyhost::call_error::PythonError;
use pyo3::exceptions::{PyAttributeError, PyImportError, PyTypeError};
use pyo3::types::{PyAnyMethods as _, PyDict, PyDictMethods as _, PyList, PyStringMethods as _};
use pyo3::{Bound, Py, PyAny, PyErr, PyResult, Python};
use secrecy::{ExposeSecret as _, SecretString};

use self::config::{apply_endpointing, recognition_keywords};
use super::error::{INSTALL_HINT, RivaCallError};
use super::options::Resolved;
use crate::api::riva::records::grpc_status::{
    CodeAttribute, DetailsAttribute, GrpcErrorObservation, grpc_error_to_exception,
};
use crate::api::riva::records::recognition::RecognitionOptions;

pub const AUTH: &str = "riva.client.Auth";
pub const RECOGNITION: &str = "riva.client.RecognitionConfig";
pub const STREAMING: &str = "riva.client.StreamingRecognitionConfig";

const CLIENT_MODULE: &str = "riva.client";
const PROTO_MODULE: &str = "riva.client.proto.riva_asr_pb2";
const RECOGNITION_ATTR: &str = "RecognitionConfig";
const FUNCTION_ID_FIELD: &str = "function-id";
const AUTHORIZATION_FIELD: &str = "authorization";
const ENCODING_FIELD: &str = "encoding";
const ENDPOINTING_FIELD: &str = "endpointing_config";
const ENCODING_ATTR: &str = "AudioEncoding";
const DEFAULT_ENCODING_ATTR: &str = "LINEAR_PCM";
const ENDPOINTING_ATTR: &str = "EndpointingConfig";
const COPY_FROM: &str = "CopyFrom";

// The optional client's handles: the package for Auth and ASRService, and whichever module exposes
// the protobuf constructors. Holds no state, so one instance can cross a blocking job.
pub struct RivaSdk {
    client: Py<PyAny>,
    asr: Py<PyAny>,
}

pub struct AuthSpec<'call> {
    pub uri: &'call str,
    pub use_ssl: bool,
    pub api_key: Option<&'call SecretString>,
    pub function_id: Option<&'call str>,
}

// The SDK objects one call shares between its two blocking jobs
pub struct Prepared {
    pub sdk: RivaSdk,
    pub auth: Py<PyAny>,
    pub recognition: Py<PyAny>,
    pub streaming: Py<PyAny>,
}

// The source's step from a resolved endpoint to the configs the stream is opened with
pub fn prepare_attached(
    py: Python<'_>,
    resolved: &Resolved,
    uri: &str,
    api_key: Option<&SecretString>,
) -> Result<Prepared, RivaCallError> {
    let sdk = RivaSdk::import(py)?;
    let spec = AuthSpec {
        uri,
        use_ssl: resolved.use_ssl,
        api_key,
        function_id: resolved.function_id.as_deref(),
    };
    let auth = sdk.auth(py, &spec)?.unbind();
    let recognition = sdk.recognition_config(py, &resolved.request.recognition_config)?;
    let streaming = sdk.streaming_config(py, &recognition)?;
    Ok(Prepared {
        sdk,
        auth,
        recognition,
        streaming,
    })
}

impl RivaSdk {
    pub fn import(py: Python<'_>) -> Result<Self, RivaCallError> {
        let client = match py.import(CLIENT_MODULE) {
            Ok(module) => module.unbind().into_any(),
            Err(error) if error.is_instance_of::<PyImportError>(py) => {
                return Err(RivaCallError::Install {
                    message: INSTALL_HINT.to_owned(),
                });
            }
            Err(error) => {
                return Err(RivaCallError::Import {
                    source: Box::new(PythonError::from_pyerr(py, &error)),
                });
            }
        };
        let asr = match client.bind(py).getattr(RECOGNITION_ATTR) {
            Ok(_constructor) => client.clone(),
            Err(error) if error.is_instance_of::<PyAttributeError>(py) => {
                match py.import(PROTO_MODULE) {
                    Ok(module) => module.unbind().into_any(),
                    Err(proto_error) if proto_error.is_instance_of::<PyImportError>(py) => {
                        return Err(RivaCallError::Install {
                            message: INSTALL_HINT.to_owned(),
                        });
                    }
                    Err(proto_error) => {
                        return Err(RivaCallError::Import {
                            source: Box::new(PythonError::from_pyerr(py, &proto_error)),
                        });
                    }
                }
            }
            Err(error) => {
                return Err(RivaCallError::Import {
                    source: Box::new(PythonError::from_pyerr(py, &error)),
                });
            }
        };
        Ok(Self { client, asr })
    }

    // Keyword arguments first, and the old positional signature only when that raises TypeError
    pub fn auth<'py>(
        &self,
        py: Python<'py>,
        spec: &AuthSpec<'_>,
    ) -> Result<Bound<'py, PyAny>, RivaCallError> {
        let factory = config_call(py, AUTH, self.client.bind(py).getattr("Auth"))?;
        let pairs = config_call(py, AUTH, PyList::new(py, metadata(spec)))?;
        let keywords = PyDict::new(py);
        config_call(py, AUTH, keywords.set_item("uri", spec.uri))?;
        config_call(py, AUTH, keywords.set_item("use_ssl", spec.use_ssl))?;
        config_call(py, AUTH, keywords.set_item("metadata_args", &pairs))?;
        match factory.call((), Some(&keywords)) {
            Ok(auth) => Ok(auth),
            Err(error) if error.is_instance_of::<PyTypeError>(py) => config_call(
                py,
                AUTH,
                factory.call1((py.None(), spec.use_ssl, spec.uri, &pairs)),
            ),
            Err(error) => Err(config_failure(py, AUTH, &error)),
        }
    }

    pub fn recognition_config(
        &self,
        py: Python<'_>,
        options: &RecognitionOptions,
    ) -> Result<Py<PyAny>, RivaCallError> {
        let module = self.asr.bind(py);
        let keywords = recognition_keywords(py, module, options)?;
        let factory = config_call(py, RECOGNITION, module.getattr(RECOGNITION_ATTR))?;
        let config = config_call(py, RECOGNITION, factory.call((), Some(&keywords)))?;
        apply_endpointing(py, module, &config, options)?;
        Ok(config.unbind())
    }

    pub fn streaming_config(
        &self,
        py: Python<'_>,
        recognition: &Py<PyAny>,
    ) -> Result<Py<PyAny>, RivaCallError> {
        let factory = config_call(
            py,
            STREAMING,
            self.asr.bind(py).getattr("StreamingRecognitionConfig"),
        )?;
        let keywords = PyDict::new(py);
        config_call(
            py,
            STREAMING,
            keywords.set_item("config", recognition.bind(py)),
        )?;
        config_call(py, STREAMING, keywords.set_item("interim_results", false))?;
        let config = config_call(py, STREAMING, factory.call((), Some(&keywords)))?;
        Ok(config.unbind())
    }

    // From here on a failure belongs to the SDK call, so it is observed and mapped as the source does
    pub fn asr_service(
        &self,
        py: Python<'_>,
        auth: &Py<PyAny>,
    ) -> Result<Py<PyAny>, RivaCallError> {
        let factory = self
            .client
            .bind(py)
            .getattr("ASRService")
            .map_err(|error| provider_failure(py, &error))?;
        let service = factory
            .call1((auth.bind(py),))
            .map_err(|error| provider_failure(py, &error))?;
        Ok(service.unbind())
    }
}

fn metadata(spec: &AuthSpec<'_>) -> Vec<(&'static str, String)> {
    let mut pairs: Vec<(&'static str, String)> = Vec::new();
    if let Some(function_id) = spec.function_id {
        pairs.push((FUNCTION_ID_FIELD, function_id.to_owned()));
    }
    if let Some(key) = spec.api_key.filter(|key| !key.expose_secret().is_empty()) {
        pairs.push((
            AUTHORIZATION_FIELD,
            format!("Bearer {}", key.expose_secret()),
        ));
    }
    pairs
}

fn config_failure(py: Python<'_>, operation: &'static str, error: &PyErr) -> RivaCallError {
    let failure = PythonError::from_pyerr(py, error);
    let source = Box::new(failure);
    RivaCallError::Config { operation, source }
}

fn config_call<T>(
    py: Python<'_>,
    operation: &'static str,
    result: PyResult<T>,
) -> Result<T, RivaCallError> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => Err(config_failure(py, operation, &error)),
    }
}

pub(super) fn provider_call<T>(py: Python<'_>, result: PyResult<T>) -> Result<T, RivaCallError> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => Err(provider_failure(py, &error)),
    }
}

pub(super) fn provider_failure(py: Python<'_>, error: &PyErr) -> RivaCallError {
    let observation = observe(py, error);
    let exception = grpc_error_to_exception(&observation);
    RivaCallError::Provider(exception)
}

fn observe(py: Python<'_>, error: &PyErr) -> GrpcErrorObservation {
    let code = code_attribute(py, error);
    let details = details_attribute(py, error);
    let text = error.value(py).str().map_or_else(
        |_text| String::new(),
        |text| text.to_string_lossy().into_owned(),
    );
    GrpcErrorObservation {
        code,
        details,
        text,
    }
}

fn code_attribute(py: Python<'_>, error: &PyErr) -> CodeAttribute {
    let Ok(attribute) = error.value(py).getattr("code") else {
        return CodeAttribute::Missing;
    };
    if !attribute.is_callable() {
        return CodeAttribute::NotCallable;
    }
    let Ok(code) = attribute.call0() else {
        return CodeAttribute::Raises;
    };
    match code
        .getattr("name")
        .and_then(|name| name.extract::<String>())
    {
        Ok(name) => CodeAttribute::Returns {
            name: Json::Text(name),
        },
        Err(_error) => CodeAttribute::Returns { name: Json::Null },
    }
}

fn details_attribute(py: Python<'_>, error: &PyErr) -> DetailsAttribute {
    let Ok(attribute) = error.value(py).getattr("details") else {
        return DetailsAttribute::Missing;
    };
    if !attribute.is_callable() {
        return DetailsAttribute::NotCallable;
    }
    let Ok(details) = attribute.call0() else {
        return DetailsAttribute::Raises;
    };
    match details.extract::<String>() {
        Ok(text) => DetailsAttribute::Returns {
            value: Json::Text(text),
        },
        Err(_error) => DetailsAttribute::Returns { value: Json::Null },
    }
}
