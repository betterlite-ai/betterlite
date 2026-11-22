use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, resolve};
use betterlite_openai::api::image_contracts::records::ImageResponse;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::image_generation_call::call::ImageGenerationCall;
use crate::api::image_generation_call::error::{
    ImageGenerationException, provider_failure, value_failure,
};

// exception_type's provider when the wrapper's own get_llm_provider set none
const FALLBACK_PROVIDER: &str = "openai";

// the ValueError aimage_generation raises for a return value it does not accept
pub(super) const INVALID_RESPONSE: &str =
    "Unable to get Image Response. Please pass a valid llm_provider.";

// The exception aimage_generation raises: the KeyError before its try, or a litellm class after it.
#[derive(Debug, Error)]
pub enum AimageGenerationError {
    // aimage_generation reads kwargs["model"] before its try, so this KeyError reaches the caller unmapped
    #[error("'model'")]
    MissingModel,
    // every failure inside the try, after exception_type has classified it
    #[error("{message}")]
    Raised { class: ErrorClass, message: String },
    // image_generation's LiteLLMUnknownProvider is a litellm exception, so exception_type hands it back as it is
    #[error("{message}")]
    UnknownProvider { message: String },
}

// get_llm_provider(model=model, api_base=kwargs.get("api_base")); its own BadRequestError is returned unchanged
pub(super) fn attribution(
    resolution: &ResolutionContext<'_>,
    redactor: &Redactor,
    call: &ImageGenerationCall,
    model: &str,
) -> Result<String, AimageGenerationError> {
    let request = ResolutionRequest {
        model,
        custom_llm_provider: None,
        api_base: call.api_base.as_deref(),
        api_key: None,
    };
    match resolve(&request, resolution) {
        Ok(found) => Ok(found.provider.slug().to_owned()),
        Err(source) => {
            // get_llm_provider raises its own BadRequestError with llm_provider "", which exception_type returns unchanged
            let raw = provider_failure(400, &source.to_string(), model, "", "BadRequestError");
            Err(classified(redactor, raw))
        }
    }
}

// a failure raised by image_generation: both litellm exception shapes reach the caller unchanged
pub(super) fn raised(
    redactor: &Redactor,
    failure: ImageGenerationException,
) -> AimageGenerationError {
    match failure {
        ImageGenerationException::Mapped { class, message } => {
            AimageGenerationError::Raised { class, message }
        }
        ImageGenerationException::UnknownProvider { message } => {
            AimageGenerationError::UnknownProvider {
                message: redactor.redact_string(&message),
            }
        }
    }
}

// ImageResponse(**mapping): a refused dict raises the wrapper's own exception under the resolved provider
pub(super) fn mapping(
    redactor: &Redactor,
    mapping: Json,
    model: &str,
    provider: &str,
) -> Result<ImageResponse, AimageGenerationError> {
    let value = serde_json::to_value(mapping)
        .map_err(|source| refused(redactor, &source.to_string(), model, provider))?;
    serde_json::from_value(value)
        .map_err(|source| refused(redactor, &source.to_string(), model, provider))
}

pub(super) fn refused(
    redactor: &Redactor,
    message: &str,
    model: &str,
    provider: &str,
) -> AimageGenerationError {
    classified(
        redactor,
        value_failure(message, Some(model), Some(attributed(provider))),
    )
}

// exception_type's own line: custom_llm_provider = custom_llm_provider or "openai"
fn attributed(provider: &str) -> &str {
    if provider.is_empty() {
        FALLBACK_PROVIDER
    } else {
        provider
    }
}

// exception_type's full dispatch over one raw exception, carrying the message the SDK record reports
fn classified(redactor: &Redactor, raw: RawException) -> AimageGenerationError {
    let message = redactor.redact_string(raw.message.as_deref().unwrap_or(&raw.error_str));
    AimageGenerationError::Raised {
        class: dispatch(&DispatchInput::Raw(Box::new(raw)), redactor),
        message,
    }
}
