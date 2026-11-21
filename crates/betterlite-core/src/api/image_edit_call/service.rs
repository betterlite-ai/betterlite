use betterlite_config::sections::image_edit_call::Settings;
use betterlite_context::media_config_selection::image_edit::{
    ImageEditConfigTag, select_image_edit,
};
use betterlite_context::non_default_params::filters::filter_out_litellm_params;
use betterlite_context::non_default_params::names::is_openai_compatible_provider;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionContext, ResolutionRequest, resolve,
};
use betterlite_openai::api::image_contracts::records::{ImageFile, ImageParams};

use super::call::{
    DEFAULT_MODEL, ImageEditCall, ImageEditClientKind, ImageEditStep, OPENAI_EDIT_PARAMS,
    request_timeout, seconds_checked,
};
use super::entry::{Attribution, ImageEditOperation};
use super::error::ImageEditError;
use super::forms::{
    control_params, form_fields, merge_form_fields, merge_non_default, merged_headers,
};
use super::logging::{ImageEditLogger, PreCallRecord};
use super::options::{params_policy, requested_options, supported_extensions};
use super::send::{
    CustomEditLookup, CustomEditProviders, CustomEditRequest, ImageEditProvider,
    ImageEditProviderSource, ImageEditSendRequest,
};

// The production operation: B174 resolution, B157 config selection, B1120 option mapping and one send.
pub struct ImageEdits<'context> {
    pub settings: &'context Settings,
    pub resolution: &'context ResolutionContext<'context>,
    pub providers: &'context dyn ImageEditProviderSource,
    pub custom: &'context dyn CustomEditProviders,
    pub global_drop_params: bool,
}

// what resolution settled before either the custom handler or a provider sees the call
struct ResolvedEdit {
    model: String,
    provider: String,
    identity: ProviderIdentity,
}

impl ImageEditOperation for ImageEdits<'_> {
    fn begin(
        &self,
        call: &ImageEditCall,
        asynchronous: bool,
        logger: &mut dyn ImageEditLogger,
    ) -> Result<ImageEditStep<'_>, ImageEditError> {
        let requested_model = call
            .model
            .clone()
            .unwrap_or_else(|| DEFAULT_MODEL.to_owned());
        // get_llm_provider runs without an api_base here, so no registered endpoint is consulted
        let resolved = resolve(
            &ResolutionRequest {
                model: &requested_model,
                custom_llm_provider: call.custom_llm_provider.as_deref(),
                api_base: None,
                api_key: None,
            },
            self.resolution,
        )?;
        let resolved = ResolvedEdit {
            model: resolved.model,
            provider: resolved.provider.slug().to_owned(),
            identity: resolved.provider,
        };
        let images = call.image.images();
        if let Some(step) = self.custom_step(call, &resolved, &images, asynchronous)? {
            return Ok(step);
        }
        self.configured_step(call, &resolved, &images, asynchronous, logger)
    }

    // image_edit rebinds model and custom_llm_provider only when get_llm_provider returns
    fn attribution(&self, call: &ImageEditCall) -> Attribution {
        let requested_model = call
            .model
            .clone()
            .unwrap_or_else(|| DEFAULT_MODEL.to_owned());
        let resolved = resolve(
            &ResolutionRequest {
                model: &requested_model,
                custom_llm_provider: call.custom_llm_provider.as_deref(),
                api_base: None,
                api_key: None,
            },
            self.resolution,
        );
        match resolved {
            Ok(resolved) => Attribution {
                model: Some(resolved.model),
                provider: Some(resolved.provider.slug().to_owned()),
            },
            Err(_unresolved) => Attribution {
                model: call.model.clone(),
                provider: call.custom_llm_provider.clone(),
            },
        }
    }
}

impl ImageEdits<'_> {
    // a listed custom provider skips config lookup, option projection and logging entirely
    fn custom_step<'this>(
        &'this self,
        call: &ImageEditCall,
        resolved: &ResolvedEdit,
        images: &[ImageFile],
        asynchronous: bool,
    ) -> Result<Option<ImageEditStep<'this>>, ImageEditError> {
        let handler = match self.custom.lookup(&resolved.provider) {
            CustomEditLookup::Missing => return Ok(None),
            CustomEditLookup::Unmatched => {
                return Err(ImageEditError::UnknownProvider {
                    model: resolved.model.clone(),
                    provider: resolved.provider.clone(),
                });
            }
            CustomEditLookup::Handler(handler) => handler,
        };
        let request = CustomEditRequest {
            model: &resolved.model,
            images,
            prompt: call.prompt.as_deref(),
            api_key: text_param(&call.kwargs, "api_key"),
            api_base: text_param(&call.kwargs, "api_base"),
            optional_params: &call.kwargs,
            timeout: call.timeout,
            client: client_for(call, asynchronous),
        };
        Ok(Some(handler.call(request, asynchronous)?))
    }

    fn configured_step<'this>(
        &'this self,
        call: &ImageEditCall,
        resolved: &ResolvedEdit,
        images: &[ImageFile],
        asynchronous: bool,
        logger: &mut dyn ImageEditLogger,
    ) -> Result<ImageEditStep<'this>, ImageEditError> {
        let tag = provider_id(&resolved.identity)
            .and_then(|provider| select_image_edit(&resolved.model, provider));
        let Some(tag) = tag else {
            return Err(ImageEditError::UnsupportedConfig {
                provider: resolved.provider.clone(),
            });
        };
        let provider =
            self.providers
                .provider(&tag)
                .ok_or_else(|| ImageEditError::UnsupportedConfig {
                    provider: resolved.provider.clone(),
                })?;
        let non_default =
            filter_out_litellm_params(&control_params(&call.kwargs), &OPENAI_EDIT_PARAMS);
        let supported = provider.supported_openai_params(&resolved.model);
        let extensions = supported_extensions(&call.kwargs, &non_default, &supported);
        let options = requested_options(call, &extensions)?;
        let policy = params_policy(call, self.global_drop_params);
        let mut params = provider.map_edit_options(&options, &resolved.model, &policy)?;
        if provider.use_multipart_form_data()
            && (resolved.provider == "openai"
                || resolved.provider == "azure"
                || is_openai_compatible_provider(&resolved.provider))
        {
            let fields = form_fields(call, &non_default)?;
            merge_form_fields(&mut params, fields);
        }
        // the record keeps the options as they stand here, before any branch-specific merge
        logger.update_from_kwargs(PreCallRecord {
            model: resolved.model.clone(),
            user: call.user.clone(),
            optional_params: params.clone(),
            litellm_params: logged_params(&params, &call.kwargs),
            custom_llm_provider: resolved.provider.clone(),
        })?;
        check_branch_model(&tag, Some(&resolved.model))?;
        let timeout = branch_timeout(&tag, call.timeout, self.settings.request_timeout_seconds)?;
        if merges_non_default(&tag) {
            merge_non_default(&mut params, &non_default)?;
        }
        let headers = merged_headers(call)?;
        provider.send(ImageEditSendRequest {
            model: &resolved.model,
            prompt: call.prompt.as_deref(),
            images,
            mask: call.mask.as_ref(),
            optional_params: &params,
            litellm_params: &call.kwargs,
            extra_headers: headers.as_ref(),
            extra_body: call.extra_body.as_ref(),
            timeout,
            api_key: text_param(&call.kwargs, "api_key"),
            api_base: text_param(&call.kwargs, "api_base"),
            custom_llm_provider: &resolved.provider,
            client: client_for(call, asynchronous),
            asynchronous,
        })
    }
}

// a supplied client of the other kind reaches no handler
fn client_for(call: &ImageEditCall, asynchronous: bool) -> Option<ImageEditClientKind> {
    call.client
        .filter(|kind| *kind == ImageEditClientKind::of(asynchronous))
}

fn logged_params(params: &ImageParams, kwargs: &ImageParams) -> ImageParams {
    let mut logged = params.clone();
    for name in ["litellm_call_id", "model_info"] {
        let value = kwargs.get(name).cloned().unwrap_or(Json::Null);
        logged.insert(name.to_owned(), value);
    }
    logged
}

fn text_param<'params>(params: &'params ImageParams, name: &str) -> Option<&'params str> {
    match params.get(name) {
        Some(Json::Text(text)) => Some(text),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

const fn provider_id(identity: &ProviderIdentity) -> Option<ProviderId> {
    match identity {
        ProviderIdentity::Known(provider) => Some(*provider),
        ProviderIdentity::Anyscale | ProviderIdentity::Custom(_) => None,
    }
}

// bedrock and BFL refuse a model of None before sending; the config path above always resolves one
pub(crate) fn check_branch_model(
    tag: &ImageEditConfigTag,
    model: Option<&str>,
) -> Result<(), ImageEditError> {
    match tag {
        ImageEditConfigTag::Bedrock(_) => require_model(model, "bedrock"),
        ImageEditConfigTag::BlackForestLabs => require_model(model, "black_forest_labs"),
        ImageEditConfigTag::OpenAi(_)
        | ImageEditConfigTag::HostedVllm(_)
        | ImageEditConfigTag::Azure
        | ImageEditConfigTag::Recraft
        | ImageEditConfigTag::FalAi(_)
        | ImageEditConfigTag::AzureAi(_)
        | ImageEditConfigTag::Gemini(_)
        | ImageEditConfigTag::LiteLlmProxy
        | ImageEditConfigTag::VertexAi(_)
        | ImageEditConfigTag::Stability(_)
        | ImageEditConfigTag::OpenRouter(_) => Ok(()),
    }
}

fn require_model(model: Option<&str>, provider: &'static str) -> Result<(), ImageEditError> {
    if model.is_some() {
        Ok(())
    } else {
        Err(ImageEditError::ModelRequired { provider })
    }
}

// timeout or DEFAULT_REQUEST_TIMEOUT everywhere but bedrock, which keeps the caller's value unchanged
pub(crate) fn branch_timeout(
    tag: &ImageEditConfigTag,
    timeout: Option<f64>,
    fallback_seconds: f64,
) -> Result<Option<f64>, ImageEditError> {
    match tag {
        ImageEditConfigTag::Bedrock(_) => timeout.map(seconds_checked).transpose(),
        ImageEditConfigTag::OpenAi(_)
        | ImageEditConfigTag::HostedVllm(_)
        | ImageEditConfigTag::Azure
        | ImageEditConfigTag::Recraft
        | ImageEditConfigTag::BlackForestLabs
        | ImageEditConfigTag::FalAi(_)
        | ImageEditConfigTag::AzureAi(_)
        | ImageEditConfigTag::Gemini(_)
        | ImageEditConfigTag::LiteLlmProxy
        | ImageEditConfigTag::VertexAi(_)
        | ImageEditConfigTag::Stability(_)
        | ImageEditConfigTag::OpenRouter(_) => request_timeout(timeout, fallback_seconds).map(Some),
    }
}

#[must_use]
pub(crate) const fn merges_non_default(tag: &ImageEditConfigTag) -> bool {
    matches!(
        tag,
        ImageEditConfigTag::Bedrock(_)
            | ImageEditConfigTag::Stability(_)
            | ImageEditConfigTag::BlackForestLabs
    )
}
