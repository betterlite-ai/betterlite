use std::future::Future;

use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::otel::proxy_runtime::slot::enter_request_slot;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::api::image_contracts::records::ImageResponse;
use betterlite_redact::secret::Redactor;

use crate::api::image_generation_call::error::ImageGenerationException;
use crate::api::image_generation_call::operations::ImageGenerationOperations;
use crate::api::image_generation_call::plan::{
    ImageGenerationPlan, ImageIdentity, image_generation,
};

pub mod error;
pub mod invocation;
mod tests;

use error::AimageGenerationError;
use invocation::{GenerationReturn, Invocation};

// image_generation as aimage_generation runs it: one call, made inside the request's own context.
pub trait ImageGenerationEntry {
    fn generate<'call>(
        &'call self,
        invocation: &Invocation,
        logging: &'call mut LoggingState,
    ) -> impl Future<Output = Result<GenerationReturn<'call>, ImageGenerationException>> + Send + 'call;
}

pub struct AimageGenerationPlan<'plan, Entry> {
    pub entry: &'plan Entry,
    pub resolution: &'plan ResolutionContext<'plan>,
    pub redactor: &'plan Redactor,
}

// The production entry: B998's image_generation always leaves its work in the coroutine.
pub struct PlanEntry<'plan, Identity, Operations> {
    pub plan: &'plan ImageGenerationPlan<'plan, Identity, Operations>,
}

impl<Identity, Operations> ImageGenerationEntry for PlanEntry<'_, Identity, Operations>
where
    Identity: ImageIdentity + Sync,
    Operations: ImageGenerationOperations + Sync,
{
    fn generate<'call>(
        &'call self,
        invocation: &Invocation,
        logging: &'call mut LoggingState,
    ) -> impl Future<Output = Result<GenerationReturn<'call>, ImageGenerationException>> + Send + 'call
    {
        let call = invocation.call.clone();
        // the coroutine is built here and only runs when the wrapper awaits it, as the sync callee does under @client
        async move {
            Ok(GenerationReturn::Coroutine(Box::pin(async move {
                image_generation(self.plan, call, logging)
                    .await
                    .map(|answer| answer.response)
            })))
        }
    }
}

// aimage_generation(*args, **kwargs): the model is read before the try, the async flag is forced, the
// provider only ever feeds exception attribution, and generation runs once inside the request's slot.
pub async fn aimage_generation<Entry>(
    plan: &AimageGenerationPlan<'_, Entry>,
    mut invocation: Invocation,
    logging: &mut LoggingState,
) -> Result<ImageResponse, AimageGenerationError>
where
    Entry: ImageGenerationEntry + Sync,
{
    let model = invocation
        .model()
        .ok_or(AimageGenerationError::MissingModel)?
        .to_owned();
    // kwargs["aimg_generation"] = True, whatever the caller passed
    invocation
        .call
        .kwargs
        .insert(String::from("aimg_generation"), Json::Bool(true));
    let provider = error::attribution(plan.resolution, plan.redactor, &invocation.call, &model)?;
    let slot = logging.request_slot.clone();
    enter_request_slot(slot, async {
        let returned = plan
            .entry
            .generate(&invocation, logging)
            .await
            .map_err(|failure| error::raised(plan.redactor, failure))?;
        match returned {
            GenerationReturn::Response(response) => Ok(*response),
            GenerationReturn::Mapping(mapping) => {
                error::mapping(plan.redactor, mapping, &model, &provider)
            }
            GenerationReturn::Coroutine(pending) => pending
                .await
                .map_err(|failure| error::raised(plan.redactor, failure)),
            GenerationReturn::Unexpected => {
                Err(error::invalid_response(plan.redactor, &model, &provider))
            }
        }
    })
    .await
}
