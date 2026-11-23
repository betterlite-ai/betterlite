use betterlite_config::loader::startup::process_environment;
use betterlite_models::catalog::BundledCatalog;
use pyo3::prelude::*;

use crate::provider::compatible::openai_compatible_info;
use crate::provider::convert::{CompatibleRequest, HandlerRequest, LookupRequest, parse};
use crate::provider::error::ProviderError;
use crate::provider::handlers::{
    handle_anthropic as anthropic_handler, handle_cohere as cohere_handler,
};
use crate::provider::lookup::{
    declared_authenticating as declared_provider, get_llm_provider as lookup_provider,
};
use crate::provider::snapshot::ProviderSnapshot;

// get_llm_provider_logic.py's functions, over one snapshot of the settings Python held at call time.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeProviders {
    snapshot: ProviderSnapshot,
}

#[pymethods]
impl NativeProviders {
    #[new]
    fn new(py: Python<'_>, settings: &str) -> PyResult<Self> {
        let snapshot = ProviderSnapshot::build_with(
            settings,
            process_environment(),
            BundledCatalog::default().get(),
        )
        .map_err(|error| error.into_pyerr(py))?;
        Ok(Self { snapshot })
    }

    fn get_llm_provider(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        let call: LookupRequest = parse(request).map_err(|error| error.into_pyerr(py))?;
        lookup_provider(&self.snapshot, &call)
            .and_then(|resolved| resolved.to_json())
            .map_err(|error| error.into_pyerr(py))
    }

    fn openai_compatible_info(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        let call: CompatibleRequest = parse(request).map_err(|error| error.into_pyerr(py))?;
        openai_compatible_info(&self.snapshot, &call)
            .and_then(|resolved| resolved.to_json())
            .map_err(|error| error.into_pyerr(py))
    }

    fn declared_authenticating(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        let call: HandlerRequest = parse(request).map_err(|error| error.into_pyerr(py))?;
        let declared = declared_provider(&call);
        serde_json::to_string(&declared.as_deref())
            .map_err(|source| ProviderError::Encode { source }.into_pyerr(py))
    }

    fn handle_cohere(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        let call: HandlerRequest = parse(request).map_err(|error| error.into_pyerr(py))?;
        let pair = cohere_handler(&self.snapshot, &call);
        serde_json::to_string(&pair)
            .map_err(|source| ProviderError::Encode { source }.into_pyerr(py))
    }

    fn handle_anthropic(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        let call: HandlerRequest = parse(request).map_err(|error| error.into_pyerr(py))?;
        let pair = anthropic_handler(&call);
        serde_json::to_string(&pair)
            .map_err(|source| ProviderError::Encode { source }.into_pyerr(py))
    }
}
