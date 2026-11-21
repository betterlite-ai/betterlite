use betterlite_context::media_optional_params::error::MediaParamsError;
use betterlite_context::media_optional_params::image_generation::{
    ImageGenerationRequest, get_optional_params_image_gen,
};
use betterlite_context::non_default_params::mapper::DropPolicy;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::resolution::ProviderIdentity;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use std::time::Duration;

use self::json::{from_values, to_values};
use self::params::{litellm_literal, litellm_params, passed_params};
use super::super::call::{GenericParams, ImageGenerationCall};
use super::super::error::{
    ImageGenerationException, provider_failure, raise, resolution_failure, unsupported_params,
    value_failure,
};
use super::super::operations::ImageGenerationOperations;
use super::{
    ImageGenerationPlan, ImageIdentity, ImageResolutionRequest, MappedOptions, PreparedCall,
    ResolvedImageModel,
};

mod json;
mod params;

const DEFAULT_MODEL: &str = "dall-e-2";

pub(super) fn prepare<Identity>(
    identity: &Identity,
    redactor: &Redactor,
    call: &ImageGenerationCall,
) -> Result<PreparedCall, ImageGenerationException>
where
    Identity: ImageIdentity,
{
    let generic = GenericParams::read(&call.kwargs).map_err(|error| {
        raise(
            redactor,
            value_failure(
                &error.to_string(),
                call.model.as_deref(),
                call.custom_llm_provider.as_deref(),
            ),
        )
    })?;
    let timeout = Duration::try_from_secs_f64(call.timeout).map_err(|source| {
        raise(
            redactor,
            value_failure(
                &format!("timeout must be a non-negative number of seconds: {source}"),
                call.model.as_deref(),
                call.custom_llm_provider.as_deref(),
            ),
        )
    })?;
    let resolved = resolve_model(identity, redactor, call)?;
    let provider_slug = resolved.provider.slug().to_owned();
    Ok(PreparedCall {
        model: resolved.model,
        provider: resolved.provider,
        provider_slug,
        api_base: resolved.api_base,
        api_key: resolved.api_key,
        timeout,
        generic,
    })
}

// `if model is not None or custom_llm_provider is not None: get_llm_provider(...)` else dall-e-2 on openai
fn resolve_model<Identity>(
    identity: &Identity,
    redactor: &Redactor,
    call: &ImageGenerationCall,
) -> Result<ResolvedImageModel, ImageGenerationException>
where
    Identity: ImageIdentity,
{
    if call.model.is_some() || call.custom_llm_provider.is_some() {
        identity
            .resolve(&ImageResolutionRequest {
                model: call.model.as_deref(),
                custom_llm_provider: call.custom_llm_provider.as_deref(),
                api_base: call.api_base.as_deref(),
                api_key: call.api_key.as_ref(),
            })
            .map_err(|error| {
                raise(
                    redactor,
                    resolution_failure(&error.to_string(), call.model.as_deref()),
                )
            })
    } else {
        Ok(ResolvedImageModel {
            model: Some(DEFAULT_MODEL.to_owned()),
            provider: ProviderIdentity::Known(ProviderId::Openai),
            api_base: call.api_base.clone(),
            api_key: call.api_key.clone(),
        })
    }
}

pub(super) fn map_options<Identity, Operations>(
    plan: &ImageGenerationPlan<'_, Identity, Operations>,
    prepared: &PreparedCall,
    call: &ImageGenerationCall,
) -> Result<MappedOptions, ImageGenerationException>
where
    Operations: ImageGenerationOperations,
{
    let model = prepared
        .generic
        .base_model
        .as_deref()
        .or(prepared.model.as_deref());
    let config = plan.operations.generation_config(model, &prepared.provider);
    let passed = passed_params(call);
    let drop_policy = DropPolicy {
        drop_params: prepared.generic.drop_params,
        litellm_drop_params: plan.drop_params,
    };
    let provider_mapper = config
        .as_ref()
        .and_then(|tag| plan.operations.options_mapper(tag));
    let values = to_values(&passed);
    let mapped = get_optional_params_image_gen(&ImageGenerationRequest {
        model,
        custom_llm_provider: Some(prepared.provider_slug.as_str()),
        params: &values,
        additional_drop_params: &prepared.generic.additional_drop_params,
        drop_policy,
        provider_config: provider_mapper.as_deref(),
        bedrock_config: plan.operations.bedrock_mapper(),
    })
    .map_err(|error| {
        project_params_error(
            plan.redactor,
            &error,
            model.unwrap_or_default(),
            &prepared.provider_slug,
        )
    })?;
    Ok(MappedOptions {
        config,
        params: from_values(mapped),
        litellm_params: litellm_params(&prepared.generic),
        headers: prepared.generic.merged_headers(),
    })
}

pub(super) fn log(
    redactor: &Redactor,
    logging: &mut LoggingState,
    prepared: &PreparedCall,
    mapped: &MappedOptions,
    call: &ImageGenerationCall,
) -> Result<(), ImageGenerationException> {
    let mut additional = IndexMap::new();
    additional.insert(
        String::from("custom_llm_provider"),
        Json::Text(prepared.provider_slug.clone()),
    );
    let update = CallUpdate {
        model: prepared.model.clone(),
        user: call.user.clone(),
        optional_params: mapped.params.clone(),
        additional_params: additional,
    };
    // the additional params always record custom_llm_provider, so LiteLLM's post-call guard never fires here
    update_from_kwargs(logging, &call.kwargs, Some(&litellm_literal(call)), update).map_err(
        |error| {
            raise(
                redactor,
                value_failure(
                    &error.to_string(),
                    prepared.model.as_deref(),
                    Some(&prepared.provider_slug),
                ),
            )
        },
    )
}

fn project_params_error(
    redactor: &Redactor,
    error: &MediaParamsError,
    model: &str,
    provider: &str,
) -> ImageGenerationException {
    match error {
        MediaParamsError::ImageUnsupported { .. }
        | MediaParamsError::TranscriptionUnsupported { .. }
        | MediaParamsError::MissingBedrockConfig => unsupported_params(&error.to_string()),
        MediaParamsError::WrongType(..)
        | MediaParamsError::Params(_)
        | MediaParamsError::Mapper(_) => raise(
            redactor,
            provider_failure(
                error.status_code(),
                &error.to_string(),
                model,
                provider,
                "ValueError",
            ),
        ),
    }
}
