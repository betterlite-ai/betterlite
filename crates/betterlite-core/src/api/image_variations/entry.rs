use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageResponse;

use super::call::{ImageVariationCall, VariationAnswer, VariationStep};
use super::error::{ImageVariationError, VariationException, project};

const DEFAULT_PROVIDER: &str = "openai";

// the seam B010 gives a provider operation, so the sync and async entries stay provider-agnostic
pub trait ImageVariationOperation {
    fn begin(&self, call: &ImageVariationCall) -> Result<VariationStep<'_>, ImageVariationError>;

    // get_llm_provider(model, api_base) in aimage_variation: read only to attribute an exception
    fn inferred_provider(&self, call: &ImageVariationCall) -> Option<String>;
}

// the sync entry: a ready provider answer passes through, and a provider future is awaited here
pub async fn image_variation<O>(
    operation: &O,
    call: &ImageVariationCall,
) -> Result<VariationAnswer, ImageVariationError>
where
    O: ImageVariationOperation,
{
    match operation.begin(call)? {
        VariationStep::Answer(answer) => Ok(answer),
        VariationStep::Pending(pending) => Ok(VariationAnswer::Response(pending.await?)),
    }
}

// forces async_call, converts one dict, awaits one coroutine, and repeats the sync call for anything else
pub async fn aimage_variation<O>(
    operation: &O,
    call: &ImageVariationCall,
) -> Result<VariationAnswer, VariationException>
where
    O: ImageVariationOperation,
{
    let mut asynchronous = call.clone();
    asynchronous.async_call = true;
    let provider = attributed_provider(operation, call);
    let first = image_variation(operation, &asynchronous)
        .await
        .map_err(|error| VariationException::mapped(&error, call.model.as_deref(), &provider))?;
    match first {
        VariationAnswer::Mapping(mapping) => {
            let response = normalized(mapping, call.model.as_deref(), &provider)?;
            Ok(VariationAnswer::Response(response))
        }
        VariationAnswer::Unexpected(_) => image_variation(operation, &asynchronous)
            .await
            .map_err(|error| VariationException::mapped(&error, call.model.as_deref(), &provider)),
        VariationAnswer::Response(response) => Ok(VariationAnswer::Response(response)),
    }
}

// custom_llm_provider or "openai", after the model-only inference get_llm_provider performs in the wrapper
fn attributed_provider<O>(operation: &O, call: &ImageVariationCall) -> String
where
    O: ImageVariationOperation,
{
    let inferred = match (&call.custom_llm_provider, &call.model) {
        (None, Some(_)) => operation.inferred_provider(call),
        _ => None,
    };
    call.custom_llm_provider
        .clone()
        .filter(|provider| !provider.is_empty())
        .or(inferred)
        .unwrap_or_else(|| DEFAULT_PROVIDER.to_owned())
}

// ImageResponse(**cached_dict): the fields convert once, and a refused dict surfaces as the wrapper's exception
fn normalized(
    mapping: Json,
    model: Option<&str>,
    provider: &str,
) -> Result<ImageResponse, VariationException> {
    let value = serde_json::to_value(mapping)
        .map_err(|error| project(&error.to_string(), None, model, provider))?;
    serde_json::from_value(value)
        .map_err(|error| project(&error.to_string(), None, model, provider))
}
