use pyo3::exceptions::PyImportError;
use pyo3::types::{PyAnyMethods as _, PyModule, PyStringMethods as _};
use pyo3::{Bound, PyAny, PyErr, PyResult, Python};

use super::error::{RivaAudioError, call_failure};

pub(super) fn import_numpy<'py>(py: Python<'py>) -> Result<Bound<'py, PyModule>, RivaAudioError> {
    match py.import("numpy") {
        Ok(module) => Ok(module),
        Err(error) if error.is_instance_of::<PyImportError>(py) => {
            Err(RivaAudioError::NumpyMissing)
        }
        Err(error) => Err(call_failure(py, "import numpy", &error)),
    }
}

pub(super) fn as_int(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<i64> {
    py.import("builtins")?
        .getattr("int")?
        .call1((value,))?
        .extract::<i64>()
}

pub(super) fn exception_text(py: Python<'_>, error: &PyErr) -> String {
    error.value(py).str().map_or_else(
        |_| String::new(),
        |text| text.to_string_lossy().into_owned(),
    )
}

pub(super) fn decode_failure(py: Python<'_>, error: &PyErr) -> RivaAudioError {
    RivaAudioError::Decode {
        message: format!(
            "Could not decode audio for Riva STT. Convert your audio to wav/flac/ogg before calling the API. Underlying error: {}",
            exception_text(py, error)
        ),
    }
}

pub(super) fn install_hint(sf_error: &str) -> RivaAudioError {
    RivaAudioError::Decode {
        message: format!(
            "Could not decode audio for Riva STT. Install audio extras (`pip install 'litellm[stt-nvidia-riva]'`) or convert your audio to wav/flac/ogg before calling the API. Underlying error: {sf_error}"
        ),
    }
}
