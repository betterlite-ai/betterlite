use pyo3::prelude::*;

pub mod a2a;
pub mod assistant_calls;
pub mod assistant_runs;
pub mod audio;
pub mod batch_calls;
pub mod batch_listing;
pub mod cache;
pub mod cache_backends;
pub mod completion;
pub mod container_calls;
pub mod container_files;
pub mod cost;
pub mod cost_extra;
pub mod decision_calls;
pub mod embeddings;
pub mod eval_calls;
pub mod eval_runs;
pub mod file_calls;
pub mod file_content;
pub mod genai;
pub mod guardrail_call;
pub mod images;
pub mod interactions;
pub mod mcp;
pub mod messages;
pub mod model_info;
pub mod moderation;
mod native_api;
pub mod passthrough_calls;
pub mod provider;
pub mod proxy_client;
pub mod rag_calls;
pub mod realtime_calls;
pub mod rerank;
pub mod response_lifecycle;
pub mod responses;
pub mod router_call;
pub mod search_ocr;
pub mod skill_calls;
pub mod tokens;
pub mod tuning_calls;
pub mod vector_files;
pub mod vector_manage;
pub mod vector_search;
pub mod video_calls;
pub mod video_edits;

fn register_first_families(module: &Bound<'_, PyModule>) -> PyResult<()> {
    a2a::register(module)?;
    assistant_calls::register(module)?;
    assistant_runs::register(module)?;
    audio::register(module)?;
    batch_calls::register(module)?;
    batch_listing::register(module)?;
    cache::register(module)?;
    cache_backends::register(module)?;
    completion::register(module)?;
    container_calls::register(module)?;
    container_files::register(module)?;
    cost::register(module)?;
    cost_extra::register(module)?;
    decision_calls::register(module)?;
    embeddings::register(module)?;
    eval_calls::register(module)?;
    eval_runs::register(module)?;
    file_calls::register(module)?;
    file_content::register(module)?;
    genai::register(module)?;
    guardrail_call::register(module)?;
    images::register(module)?;
    interactions::register(module)?;
    Ok(())
}

fn register_last_families(module: &Bound<'_, PyModule>) -> PyResult<()> {
    mcp::register(module)?;
    messages::register(module)?;
    model_info::register(module)?;
    moderation::register(module)?;
    passthrough_calls::register(module)?;
    provider::register(module)?;
    proxy_client::register(module)?;
    rag_calls::register(module)?;
    realtime_calls::register(module)?;
    rerank::register(module)?;
    response_lifecycle::register(module)?;
    responses::register(module)?;
    router_call::register(module)?;
    search_ocr::register(module)?;
    skill_calls::register(module)?;
    tokens::register(module)?;
    tuning_calls::register(module)?;
    vector_files::register(module)?;
    vector_manage::register(module)?;
    vector_search::register(module)?;
    video_calls::register(module)?;
    video_edits::register(module)?;
    Ok(())
}

#[pymodule]
mod _native {
    use std::collections::BTreeMap;

    use pyo3::exceptions::PyRuntimeError;
    use pyo3::prelude::*;

    use super::{native_api, register_first_families, register_last_families};

    #[pyfunction]
    pub(super) const fn core_version() -> &'static str {
        // The logic stays in native_api so its tests need no interpreter.
        native_api::core_version()
    }

    #[pyfunction]
    pub(super) fn bundled_model_cost_json() -> PyResult<String> {
        native_api::bundled_model_cost_json()
            .map_err(|error| PyRuntimeError::new_err(error.to_string()))
    }

    #[pyfunction]
    pub(super) fn provider_model_sets() -> BTreeMap<String, Vec<String>> {
        // The logic stays in native_api so its tests need no interpreter.
        native_api::provider_model_sets()
    }

    #[pymodule_init]
    fn init(module: &Bound<'_, PyModule>) -> PyResult<()> {
        register_first_families(module)?;
        register_last_families(module)
    }
}
