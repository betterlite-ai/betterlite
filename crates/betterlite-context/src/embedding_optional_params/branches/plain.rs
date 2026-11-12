use super::BranchOutcome;
use super::NO_SUPPORTED_PARAMS;
use super::mapping::delegating_params;
use super::supported::reject_unsupported;
use crate::embedding_optional_params::error::EmbeddingParamsError;
use crate::embedding_optional_params::request::EmbeddingRequest;
use crate::non_default_params::Params;
use crate::non_default_params::names::is_openai_compatible_provider;

pub(super) fn plain_params(
    request: &EmbeddingRequest<'_>,
    provider: &str,
    non_default_params: &Params,
    dropped: bool,
) -> Result<BranchOutcome, EmbeddingParamsError> {
    if is_delegating_provider(provider) {
        let mapped = delegating_params(request, non_default_params, request.supported, dropped)?;
        return Ok(BranchOutcome::Mapped(mapped));
    }
    if provider == "azure" || is_openai_compatible_provider(provider) {
        return Ok(BranchOutcome::Mapped(non_default_params.clone()));
    }
    reject_unsupported(
        request,
        non_default_params,
        Some(&NO_SUPPORTED_PARAMS),
        dropped,
    )?;
    Ok(BranchOutcome::Mapped(Params::new()))
}

fn is_delegating_provider(provider: &str) -> bool {
    matches!(
        provider,
        "triton"
            | "databricks"
            | "nvidia_nim"
            | "mistral"
            | "jina_ai"
            | "sap"
            | "fireworks_ai"
            | "sambanova"
            | "ovhcloud"
    )
}
