use std::time::Duration;

use betterlite_config::sections::image_variations::Settings;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::identity::azure::is_non_openai_azure_model;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionContext, ResolutionRequest, resolve,
};
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageInputTokens, ImageParams, ImageResponse, ImageUsage,
};
use betterlite_openai::image_variations::logging::{
    ImageVariationLogger, PostCallRecord, PreCallRecord,
};
use betterlite_openai::image_variations::request::{
    ImageVariationOutcome, begin_image_variation, finish_image_variation, send_image_variation,
};
use betterlite_openai::image_variations::transport::ImageVariationTransport;
use betterlite_openai::image_variations::{
    ImageVariationParams, ImageVariationRequest, OpenAiImageVariationConfig,
};
use secrecy::SecretString;
use uuid::Uuid;

use super::call::{DEFAULT_MODEL, ImageVariationCall, VariationStep};
use super::credentials::VariationPolicies;
use super::entry::ImageVariationOperation;
use super::error::ImageVariationError;
use super::topaz::{TopazVariationExecutor, TopazVariationRequest};

pub struct ImageVariations<'context, Transport, Topaz> {
    pub settings: &'context Settings,
    pub resolution: &'context ResolutionContext<'context>,
    pub transport: &'context Transport,
    pub topaz: &'context Topaz,
    pub policies: VariationPolicies,
    pub identify: fn() -> Uuid,
    pub clock: fn() -> i64,
}

impl<Transport, Topaz> ImageVariationOperation for ImageVariations<'_, Transport, Topaz>
where
    Transport: ImageVariationTransport,
    Topaz: TopazVariationExecutor,
{
    fn begin(&self, call: &ImageVariationCall) -> Result<VariationStep<'_>, ImageVariationError> {
        let model = declared_model(call, self.resolution.catalog);
        let request = ResolutionRequest {
            model: &model,
            // resolve treats an empty declaration as absent, as get_llm_provider's falsy check does
            custom_llm_provider: call.custom_llm_provider.as_deref(),
            api_base: call.api_base.as_deref(),
            api_key: call.api_key.as_ref(),
        };
        let resolved =
            resolve(&request, self.resolution).map_err(ImageVariationError::Resolution)?;
        let provider = VariationProvider::of(&resolved.provider)?;
        if (self.policies.model_info)(Some(resolved.model.as_str()), provider.id()).is_none() {
            return Err(ImageVariationError::UnknownModelInfo {
                provider: resolved.provider.slug().to_owned(),
            });
        }
        let credentials = (self.policies.credentials)(
            provider.id(),
            call.api_key.as_ref(),
            call.api_base.as_deref(),
            self.settings,
        );
        let api_key = credentials
            .api_key
            .ok_or_else(|| ImageVariationError::MissingKey {
                provider: provider.name(),
            })?;
        let api_base = credentials
            .api_base
            .ok_or_else(|| ImageVariationError::MissingBase {
                provider: provider.name(),
            })?;
        self.dispatch(call, resolved.model, api_key, api_base, provider)
    }

    fn inferred_provider(&self, call: &ImageVariationCall) -> Option<String> {
        let model = call.model.as_deref()?;
        let request = ResolutionRequest {
            model,
            custom_llm_provider: None,
            api_base: call.api_base.as_deref(),
            api_key: None,
        };
        resolve(&request, self.resolution)
            .ok()
            .map(|resolved| resolved.provider.slug().to_owned())
    }
}

impl<Transport, Topaz> ImageVariations<'_, Transport, Topaz>
where
    Transport: ImageVariationTransport,
    Topaz: TopazVariationExecutor,
{
    fn dispatch(
        &self,
        call: &ImageVariationCall,
        model: String,
        api_key: SecretString,
        api_base: String,
        provider: VariationProvider,
    ) -> Result<VariationStep<'_>, ImageVariationError> {
        match provider {
            VariationProvider::OpenAi => {
                let request = ImageVariationRequest {
                    model: Some(model),
                    image: call.image.clone(),
                    // the signature's n, response_format, size and user stop here
                    optional_params: ImageParams::new(),
                    headers: None,
                    api_key: Some(api_key),
                    api_base: Some(api_base),
                    organization: None,
                    timeout: call
                        .timeout
                        .map(|seconds| {
                            Duration::try_from_secs_f64(seconds)
                                .map_err(|_invalid| ImageVariationError::InvalidTimeout { seconds })
                        })
                        .transpose()?,
                    params: ImageVariationParams {
                        async_call: call.async_call,
                    },
                    client: None,
                    shared_session: None,
                    config: Some(OpenAiImageVariationConfig),
                    boundary: (self.identify)().to_string(),
                    now_seconds: (self.clock)(),
                };
                let prepared = begin_image_variation(request, &mut VariationLogger)?;
                let transport: &Transport = self.transport;
                Ok(VariationStep::Pending(Box::pin(async move {
                    let sent = send_image_variation(&prepared, transport).await;
                    let outcome = finish_image_variation(&prepared, sent, &mut VariationLogger)?;
                    Ok(image_response(outcome))
                })))
            }
            VariationProvider::Topaz => {
                let request = TopazVariationRequest {
                    model,
                    image: call.image.clone(),
                    optional_params: ImageParams::new(),
                    api_key,
                    api_base,
                    timeout_seconds: topaz_timeout(
                        call.timeout,
                        self.settings.request_timeout_seconds,
                    ),
                    async_call: call.async_call,
                    metadata: call.metadata.clone(),
                };
                match self.topaz.execute(request)? {
                    Some(pending) => Ok(VariationStep::Pending(pending)),
                    None => Err(ImageVariationError::InvalidProvider {
                        provider: provider.id().to_string(),
                    }),
                }
            }
        }
    }
}

// get_llm_provider's line 207 prefix runs after its azure non-openai branch, whose model keeps its prefix
fn declared_model(call: &ImageVariationCall, catalog: &ModelCatalog) -> String {
    let model = call
        .model
        .clone()
        .unwrap_or_else(|| DEFAULT_MODEL.to_owned());
    let Some(provider) = call
        .custom_llm_provider
        .as_deref()
        .filter(|provider| !provider.is_empty())
    else {
        return model;
    };
    let azure_branch =
        model.split('/').next() == Some("azure") && is_non_openai_azure_model(&model, catalog);
    if azure_branch || model.split('/').next() == Some(provider) {
        return model;
    }
    format!("{provider}/{model}")
}

// litellm_params.timeout or DEFAULT_REQUEST_TIMEOUT: a missing or zero timeout takes the configured value
fn topaz_timeout(timeout: Option<f64>, request_timeout_seconds: f64) -> f64 {
    match timeout {
        Some(seconds) if seconds != 0.0 => seconds,
        _ => request_timeout_seconds,
    }
}

// the handler builds ImageResponse(**response_json) over the reply: litellm's ImageUsage defaults to zeros
fn image_response(outcome: ImageVariationOutcome) -> ImageResponse {
    let reply = outcome.reply;
    let usage = ImageUsage {
        input_tokens: reply.usage.input_tokens,
        input_tokens_details: ImageInputTokens {
            image_tokens: reply.usage.input_tokens_details.image_tokens,
            text_tokens: reply.usage.input_tokens_details.text_tokens,
        },
        output_tokens: reply.usage.output_tokens,
        total_tokens: reply.usage.total_tokens,
    };
    let data = reply
        .data
        .into_iter()
        .map(|datum| ImageData {
            b64_json: datum.b64_json,
            revised_prompt: datum.revised_prompt,
            url: datum.url,
        })
        .collect();
    ImageResponse {
        created: reply.created,
        data: Some(data),
        usage: Some(usage),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VariationProvider {
    OpenAi,
    Topaz,
}

impl VariationProvider {
    fn of(identity: &ProviderIdentity) -> Result<Self, ImageVariationError> {
        match identity {
            ProviderIdentity::Known(ProviderId::Openai) => Ok(Self::OpenAi),
            ProviderIdentity::Known(ProviderId::Topaz) => Ok(Self::Topaz),
            other => Err(ImageVariationError::InvalidProvider {
                provider: other.slug().to_owned(),
            }),
        }
    }

    fn id(self) -> ProviderId {
        match self {
            Self::OpenAi => ProviderId::Openai,
            Self::Topaz => ProviderId::Topaz,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI",
            Self::Topaz => "Topaz",
        }
    }
}

// image_variation logs through the caller's logging object; this entry has none ready, so it records nothing
struct VariationLogger;

impl ImageVariationLogger for VariationLogger {
    fn pre_call(&mut self, _record: PreCallRecord) -> Result<(), PrePostError> {
        Ok(())
    }

    fn post_call(&mut self, _record: PostCallRecord) {}
}
