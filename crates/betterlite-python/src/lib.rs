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

type FamilyRegister = fn(&Bound<'_, PyModule>) -> PyResult<()>;

// The one explicit family list: a family task fills its stub and never edits these lines.
const FAMILY_REGISTERS: [FamilyRegister; 45] = [
    a2a::register,
    assistant_calls::register,
    assistant_runs::register,
    audio::register,
    batch_calls::register,
    batch_listing::register,
    cache::register,
    cache_backends::register,
    completion::register,
    container_calls::register,
    container_files::register,
    cost::register,
    cost_extra::register,
    decision_calls::register,
    embeddings::register,
    eval_calls::register,
    eval_runs::register,
    file_calls::register,
    file_content::register,
    genai::register,
    guardrail_call::register,
    images::register,
    interactions::register,
    mcp::register,
    messages::register,
    model_info::register,
    moderation::register,
    passthrough_calls::register,
    provider::register,
    proxy_client::register,
    rag_calls::register,
    realtime_calls::register,
    rerank::register,
    response_lifecycle::register,
    responses::register,
    router_call::register,
    search_ocr::register,
    skill_calls::register,
    tokens::register,
    tuning_calls::register,
    vector_files::register,
    vector_manage::register,
    vector_search::register,
    video_calls::register,
    video_edits::register,
];

#[pymodule]
mod _native {
    use std::collections::BTreeMap;

    use pyo3::exceptions::PyRuntimeError;
    use pyo3::prelude::*;

    use super::{FAMILY_REGISTERS, native_api};

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
        for register in FAMILY_REGISTERS {
            register(module)?;
        }
        Ok(())
    }
}
