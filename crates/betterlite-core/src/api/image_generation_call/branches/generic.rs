use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;

use super::super::call::ImageGenerationCall;
use super::super::error::{ImageGenerationException, raise, value_failure};
use super::super::operations::{Branch, BranchAnswer, GenericRequest, ImageGenerationOperations};
use super::super::plan::{ImageGenerationPlan, MappedOptions, PreparedCall};

pub(super) async fn send<Identity, Operations>(
    plan: &ImageGenerationPlan<'_, Identity, Operations>,
    prepared: &PreparedCall,
    mapped: &MappedOptions,
    call: &ImageGenerationCall,
    logging: &mut LoggingState,
) -> Result<BranchAnswer, ImageGenerationException>
where
    Identity: Sync,
    Operations: ImageGenerationOperations + Sync,
{
    let Some(config) = mapped.config.clone() else {
        let slug = prepared.provider_slug.as_str();
        let message = format!("image generation config is not supported for {slug}");
        let failure = value_failure(&message, prepared.model.as_deref(), Some(slug));
        return Err(raise(plan.redactor, failure));
    };
    // this branch alone writes the global api_base fallback into litellm_params before the handler runs
    let api_base = prepared
        .api_base
        .clone()
        .or_else(|| plan.settings.api_base.clone());
    let mut litellm_params = mapped.litellm_params.clone();
    litellm_params.insert(
        String::from("api_base"),
        api_base.map_or(Json::Null, Json::Text),
    );
    plan.operations
        .send(
            Branch::Generic(GenericRequest {
                config,
                provider: &prepared.provider_slug,
                model: prepared.model.as_deref(),
                prompt: &call.prompt,
                optional_params: &mapped.params,
                litellm_params: &litellm_params,
                api_key: call.api_key.as_ref(),
                timeout: prepared.timeout,
            }),
            logging,
        )
        .await
        .map_err(|failure| raise(plan.redactor, *failure))
}
