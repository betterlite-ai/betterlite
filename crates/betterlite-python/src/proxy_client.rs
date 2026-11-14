use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_secrets::gateway_key::get_litellm_gateway_api_key;

use self::token::TokenFileStore;

mod tests;
mod token;

#[pyclass]
struct ProxyClientNative {
    redactor: Redactor,
}

#[pymethods]
impl ProxyClientNative {
    #[new]
    fn new() -> PyResult<Self> {
        Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
            .map(|redactor| Self { redactor })
            .map_err(|error| PyRuntimeError::new_err(error.to_string()))
    }

    fn redact_text(&self, value: &str) -> String {
        // betterlite-redact owns the patterns, so the wheel has one implementation.
        self.redactor.redact_string(value)
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
