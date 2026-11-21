use betterlite_logging::logging_state::LoggingState;
use betterlite_redact::secret::Redactor;

use super::super::call::ImageGenerationCall;
use super::super::error::{ImageGenerationException, raise, value_failure};
use super::super::operations::{
    BedrockRequest, BlackForestLabsRequest, Branch, BranchAnswer, ImageGenerationOperations,
};
use super::super::plan::{ImageGenerationPlan, MappedOptions, PreparedCall};

// The two handlers that reject a call without a model before any provider work
pub(super) async fn send<Identity, Operations>(
    plan: &ImageGenerationPlan<'_, Identity, Operations>,
    prepared: &PreparedCall,
    mapped: &MappedOptions,
    call: &ImageGenerationCall,
    logging: &mut LoggingState,
) -> Result<BranchAnswer, ImageGenerationException>
where
    Operations: ImageGenerationOperations + Sync,
{
    let provider = prepared.provider_slug.as_str();
    let model = required_model(plan.redactor, prepared, provider)?;
    let branch = if provider == "black_forest_labs" {
        Branch::BlackForestLabs(BlackForestLabsRequest {
            model,
            prompt: &call.prompt,
            optional_params: &mapped.params,
            litellm_params: &mapped.litellm_params,
            timeout: prepared.timeout,
            extra_headers: prepared.generic.extra_headers.as_ref(),
            asynchronous: prepared.generic.aimg_generation,
        })
    } else {
        Branch::Bedrock(BedrockRequest {
            model: Some(model),
            prompt: &call.prompt,
            timeout: prepared.timeout,
            api_key: call.api_key.as_ref(),
            api_base: prepared.api_base.as_deref(),
            optional_params: &mapped.params,
            asynchronous: prepared.generic.aimg_generation,
        })
    };
    plan.operations
        .send(branch, logging)
        .await
        .map_err(|failure| raise(plan.redactor, *failure))
}

// The Model needs to be set exception both handlers raise with their provider slug
fn required_model<'prepared>(
    redactor: &Redactor,
    prepared: &'prepared PreparedCall,
    provider: &str,
) -> Result<&'prepared str, ImageGenerationException> {
    let Some(model) = prepared.model.as_deref() else {
        let message = format!("Model needs to be set for {provider}");
        let failure = value_failure(&message, None, Some(provider));
        return Err(raise(redactor, failure));
    };
    Ok(model)
}
