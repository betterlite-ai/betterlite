use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyString;

use betterlite_secrets::gateway_key::get_litellm_gateway_api_key;

use self::redact::TextRedactor;
use self::token::TokenFileStore;

mod redact;
mod tests;
mod token;

#[pyclass]
struct ProxyClientNative {
    redactor: TextRedactor,
}

#[pymethods]
impl ProxyClientNative {
    #[new]
    fn new() -> PyResult<Self> {
        let redactor = TextRedactor::from_defaults()
            .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
        Ok(Self { redactor })
    }

    fn redact_text<'py>(&self, py: Python<'py>, value: &str) -> Bound<'py, PyString> {
        let redacted = self.redactor.redact(value);
        PyString::new(py, &redacted)
    }

    fn gateway_api_key(&self, expected_base_url: Option<String>) -> Option<String> {
        let home = std::env::home_dir()?;
        let store = TokenFileStore {
            token_file: home.join(".litellm").join("token.json"),
        };
        get_litellm_gateway_api_key(expected_base_url.as_deref(), &store)
    }
}

pub fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<ProxyClientNative>()?;
    Ok(())
}
