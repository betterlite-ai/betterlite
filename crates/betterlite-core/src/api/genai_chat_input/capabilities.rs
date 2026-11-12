use std::collections::BTreeSet;

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::lookup::capabilities::{CapabilityFlag, CapabilityView};
use betterlite_models::params::CallType;
use betterlite_models::params::capabilities::{
    CapabilityQuery, ModelProjection, OpenAiCapability, OpenAiConfigTag, ParameterCapabilities,
    supported_parameters,
};
use betterlite_models::resolution::{ProviderIdentity, ResolutionRequest, ResolvedModel, resolve};
use betterlite_openai::chat::ResponseFormat;
use betterlite_openai::openai_base_config::{CatalogCapabilities, ModelFamily, model_family};

use super::GenAiInputError;
use super::TranslationContext;
use super::input::{GenerateContentRequest, GenerateLiteLLMParams};
use super::response_format::response_format;

pub(super) fn supported_response_format(
    request: &GenerateContentRequest,
    litellm_params: Option<&GenerateLiteLLMParams>,
    context: &TranslationContext<'_>,
) -> Result<Option<ResponseFormat>, GenAiInputError> {
    let mut format = response_format(request.generation_config.as_ref(), &context.limits)?;
    if format.is_some() && !supports_response_format(request, litellm_params, context) {
        format = None;
    }
    Ok(format)
}

fn supports_response_format(
    request: &GenerateContentRequest,
    litellm_params: Option<&GenerateLiteLLMParams>,
    context: &TranslationContext<'_>,
) -> bool {
    let custom = litellm_params.and_then(|params| params.custom_llm_provider.as_deref());
    let resolution = ResolutionRequest {
        model: &request.model,
        // an empty declaration names no provider, as LiteLLM's falsy check treats it
        custom_llm_provider: custom.filter(|name| !name.is_empty()),
        api_base: None,
        api_key: None,
    };
    let Ok(ResolvedModel {
        provider: ProviderIdentity::Known(provider),
        model,
        ..
    }) = resolve(&resolution, context.resolution)
    else {
        // LiteLLM keeps response_format when get_llm_provider raises BadRequestError
        return true;
    };
    let capabilities = response_schema_capabilities(context.resolution.catalog, &model);
    let query = CapabilityQuery {
        provider,
        call_type: CallType::ChatCompletion,
        config: openai_config_tag(&model, context.model_capabilities),
        model: ModelProjection {
            model: &model,
            capabilities: &capabilities,
        },
        base_model: None,
    };
    match supported_parameters(&query) {
        // no parameter list describes a provider betterlite has not built, and LiteLLM keeps the field
        ParameterCapabilities::Absent => true,
        ParameterCapabilities::Listed(names) => names.contains(&"response_format"),
    }
}

fn openai_config_tag(model: &str, catalog: &CatalogCapabilities) -> OpenAiConfigTag {
    match model_family(model, catalog) {
        ModelFamily::OSeries => OpenAiConfigTag::OSeries,
        ModelFamily::GptReasoning => OpenAiConfigTag::Gpt5,
        ModelFamily::Audio | ModelFamily::Gpt => OpenAiConfigTag::Gpt,
        ModelFamily::Unknown => OpenAiConfigTag::UnknownModel,
    }
}

fn response_schema_capabilities(catalog: &ModelCatalog, model: &str) -> BTreeSet<OpenAiCapability> {
    let mut capabilities = BTreeSet::new();
    let found = catalog
        .lookup
        .get(model)
        .and_then(|canonical| catalog.entries.get(canonical.as_ref()));
    if let Some(entry) = found
        && CapabilityView::from_fields(&entry.fields.capabilities)
            .get(CapabilityFlag::ResponseSchema)
            == Some(true)
    {
        capabilities.insert(OpenAiCapability::ResponseSchema);
    }
    capabilities
}
