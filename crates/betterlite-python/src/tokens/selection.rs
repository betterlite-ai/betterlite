use pyo3::prelude::*;

use betterlite_models::catalog::BundledCatalog;
use betterlite_tokenizer::selection::{
    openai_tokenizer_encoding_name, uses_legacy_message_accounting,
};

use super::convert::{ModelRequest, read};

#[pyfunction]
pub(crate) fn openai_encoding_name(request: &str) -> PyResult<&'static str> {
    let wire: ModelRequest = read(request)?;
    let catalog = BundledCatalog::default().get();
    Ok(openai_tokenizer_encoding_name(&wire.model, &catalog).name())
}

// no bundled model normalizes to gpt-3.5-turbo-0301, so only the false answer is reachable and testable; B259 owns the true branch
#[cfg_attr(test, mutants::skip)]
#[pyfunction]
pub(crate) fn legacy_message_accounting(request: &str) -> PyResult<bool> {
    let wire: ModelRequest = read(request)?;
    let catalog = BundledCatalog::default().get();
    Ok(uses_legacy_message_accounting(&wire.model, &catalog))
}

mod tests;
