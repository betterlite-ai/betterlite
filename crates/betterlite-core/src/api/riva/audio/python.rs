use pyo3::exceptions::PyImportError;
use pyo3::types::{PyAnyMethods as _, PyModule, PyStringMethods as _};
use pyo3::{Bound, PyAny, PyErr, PyResult, Python};

use super::error::{RivaAudioError, python_error};

/// Imports numpy the way the source does: an ImportError becomes the install hint, anything else
/// stays a typed Python failure.
pub(super) fn import_numpy<'py>(py: Python<'py>) -> Result<Bound<'py, PyModule>, RivaAudioError> {
    match py.import("numpy") {
        Ok(module) => Ok(module),
        Err(error) if error.is_instance_of::<PyImportError>(py) => {
            Err(RivaAudioError::NumpyMissing)
        }
        Err(error) => Err(python_error(py, "import numpy", &error)),
    }
}

/// The source's `int(value)` call, spelled the same way so numpy integers narrow like they do there.
pub(super) fn as_int(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<i64> {
    py.import("builtins")?
        .getattr("int")?
        .call1((value,))?
        .extract::<i64>()
}

/// `str(exception)`, which is what the source formats into its decode errors.
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
