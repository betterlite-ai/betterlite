use betterlite_azure::credentials::source::CallableTokenProvider;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::image_generation_call::Settings;
use betterlite_context::media_config_selection::image_generation::ImageGenerationConfigTag;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionContext, ResolutionError, ResolutionRequest, resolve,
};
use betterlite_openai::api::image_contracts::records::ImageResponse;
use betterlite_redact::secret::Redactor;
use secrecy::SecretString;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use stages::{log, map_options, prepare};

use super::branches::send_branch;
use super::call::{GenericParams, ImageGenerationCall};
use super::error::ImageGenerationException;
use super::mock::mock_answer;
use super::operations::{CustomProviderSlot, Headers, ImageGenerationOperations, Params};

mod stages;

pub(super) struct PreparedCall {
    pub(super) model: Option<String>,
    pub(super) provider: ProviderIdentity,
    pub(super) provider_slug: String,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<SecretString>,
    pub(super) timeout: Duration,
    pub(super) generic: GenericParams,
}

pub(super) struct MappedOptions {
    pub(super) config: Option<ImageGenerationConfigTag>,
    pub(super) params: Params,
    pub(super) litellm_params: Params,
    pub(super) headers: Headers,
}

pub struct ImageResolutionRequest<'request> {
    pub model: Option<&'request str>,
    pub custom_llm_provider: Option<&'request str>,
    pub api_base: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
}

#[derive(Debug, Clone)]
pub struct ResolvedImageModel {
    pub model: Option<String>,
    pub provider: ProviderIdentity,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum ImageResolutionError {
    #[error("{message}")]
    Rejected { message: String },
    #[error("model parameter is required but was None. Please provide a valid model name.")]
    MissingModel,
}

pub trait ImageIdentity {
    fn resolve(
        &self,
        request: &ImageResolutionRequest<'_>,
    ) -> Result<ResolvedImageModel, ImageResolutionError>;
}

pub struct CatalogIdentity<'context, 'borrow> {
    pub context: &'borrow ResolutionContext<'context>,
}

impl ImageIdentity for CatalogIdentity<'_, '_> {
    fn resolve(
        &self,
        request: &ImageResolutionRequest<'_>,
    ) -> Result<ResolvedImageModel, ImageResolutionError> {
        let Some(model) = request.model else {
            return Err(ImageResolutionError::MissingModel);
        };
        let resolved = resolve(
            &ResolutionRequest {
                model,
                custom_llm_provider: request.custom_llm_provider,
                api_base: request.api_base,
                api_key: request.api_key,
            },
            self.context,
        )
        .map_err(|error: ResolutionError| ImageResolutionError::Rejected {
            message: error.to_string(),
        })?;
        Ok(ResolvedImageModel {
            model: Some(resolved.model),
            provider: resolved.provider,
            api_base: resolved.api_base,
            api_key: resolved.api_key,
        })
    }
}

pub struct ImageGenerationPlan<'plan, Identity, Operations> {
    pub identity: &'plan Identity,
    pub operations: &'plan Operations,
    pub settings: &'plan Settings,
    pub environment: &'plan dyn EnvironmentLookup,
    pub custom_providers: &'plan BTreeSet<String>,
    pub custom_provider_map: &'plan [CustomProviderSlot],
    pub azure_ad_token_provider: Option<Arc<dyn CallableTokenProvider>>,
    pub drop_params: bool,
    pub redactor: &'plan Redactor,
}

// What the SDK reports beside the response: the model LiteLLM keeps in _hidden_params
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageGenerationAnswer {
    pub response: ImageResponse,
    pub hidden_model: Option<String>,
}

pub async fn image_generation<Identity, Operations>(
    plan: &ImageGenerationPlan<'_, Identity, Operations>,
    call: ImageGenerationCall,
    logging: &mut LoggingState,
) -> Result<ImageGenerationAnswer, ImageGenerationException>
where
    Identity: ImageIdentity + Sync,
    Operations: ImageGenerationOperations + Sync,
{
    let prepared = prepare(plan.identity, plan.redactor, &call)?;
    let mapped = map_options(plan, &prepared, &call)?;
    log(plan.redactor, logging, &prepared, &mapped, &call)?;
    // mock_image_generation builds a fresh ImageResponse, so the mock records no hidden model
    if let Some(mocked) = mock_answer(&prepared.generic, &call) {
        return Ok(ImageGenerationAnswer {
            response: mocked.response,
            hidden_model: mocked.hidden_model,
        });
    }
    let answer = send_branch(plan, &prepared, &mapped, &call, logging).await?;
    Ok(ImageGenerationAnswer {
        response: answer.response,
        hidden_model: answer.hidden_model.or_else(|| prepared.model.clone()),
    })
}
