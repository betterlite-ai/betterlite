use super::backend::HealthProbeBackend;
use super::calls::{HealthProbeReply, probe_call};
use super::error::HealthProbeError;
use super::logging::ProbeLoggingUpdate;
use super::params::HealthProbeParams;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
pub const LIST_BATCHES_SUPPORTED_PROVIDERS: [&str; 6] = [
    "openai",
    "azure",
    "hosted_vllm",
    "litellm_proxy",
    "vertex_ai",
    "xai",
];
pub async fn probe_batch_mode(
    custom_llm_provider: &str,
    params: &HealthProbeParams,
    backend: &dyn HealthProbeBackend,
) -> Result<HealthProbeReply, HealthProbeError> {
    let filtered = params.call_parameters(false);
    update_logging(params, &filtered)?;
    if !LIST_BATCHES_SUPPORTED_PROVIDERS.contains(&custom_llm_provider) {
        return backend
            .chat(probe_call(params.call_parameters(true), params))
            .await;
    }
    let mut parameters = filtered;
    parameters.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(custom_llm_provider.to_owned()),
    );
    backend.batch(probe_call(parameters, params)).await
}
fn update_logging(
    params: &HealthProbeParams,
    filtered: &IndexMap<String, Json>,
) -> Result<(), HealthProbeError> {
    let Some(logging) = &params.litellm_logging_obj else {
        return Ok(());
    };
    let api_base = params.api_base.as_option().filter(|text| !text.is_empty());
    let litellm_params = match api_base {
        Some(base) => {
            let mut entries = IndexMap::new();
            entries.insert("api_base".to_owned(), Json::Text(base.clone()));
            Some(entries)
        }
        None => None,
    };
    let update = ProbeLoggingUpdate {
        kwargs: filtered.clone(),
        model: params.model.as_option().cloned(),
        user: None,
        optional_params: IndexMap::new(),
        litellm_params,
    };
    logging
        .update_from_kwargs(update)
        .map_err(|source| HealthProbeError::Logging { source })
}
