use betterlite_logging::logging_state::LoggingState;
use std::sync::Arc;

use super::super::call::{CustomClientKind, ImageGenerationCall};
use super::super::error::{ImageGenerationException, raise, unknown_provider};
use super::super::operations::{
    BranchAnswer, CustomImageHandler, CustomRequest, ImageGenerationOperations,
};
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
    // LiteLLM keeps overwriting the handler, so the last matching map entry is the one that runs
    let mut selected: Option<Arc<dyn CustomImageHandler>> = None;
    for slot in plan.custom_provider_map {
        if slot.provider == prepared.provider_slug {
            selected.clone_from(&slot.handler);
        }
    }
    let Some(handler) = selected else {
        return Err(unknown_provider(
            prepared.model.as_deref(),
            &prepared.provider_slug,
        ));
    };
    // each leg only accepts its own client class; the other kind becomes None
    let client = if prepared.generic.aimg_generation {
        (call.client_kind == Some(CustomClientKind::Asynchronous))
            .then_some(CustomClientKind::Asynchronous)
    } else {
        (call.client_kind == Some(CustomClientKind::Sync)).then_some(CustomClientKind::Sync)
    };
    handler
        .generate(
            CustomRequest {
                provider: &prepared.provider_slug,
                model: prepared.model.as_deref(),
                prompt: &call.prompt,
                api_key: call.api_key.as_ref(),
                api_base: prepared.api_base.as_deref(),
                optional_params: &mapped.params,
                timeout: prepared.timeout,
                asynchronous: prepared.generic.aimg_generation,
                client,
            },
            logging,
        )
        .await
        .map_err(|failure| raise(plan.redactor, *failure))
}
