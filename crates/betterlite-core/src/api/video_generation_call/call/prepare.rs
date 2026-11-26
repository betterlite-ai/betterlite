use std::str::FromStr as _;
use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, ResolvedModel, resolve};
use indexmap::IndexMap;

use super::super::error::{VideoGenerationCallError, VideoGenerationFailure, failure};
use super::super::handler::VideoGenerationHandling;
use super::super::logging::{LoggedGeneration, log_call};
use super::super::options::{provider_request_params, requested_optional_params};
use super::super::providers::VideoGenerationProviders;
use super::super::{DEFAULT_MODEL, LITELLM_CALL_ID, VideoGenerationRequest};
use super::VideoGenerationCaller;
use crate::api::video_selection::{VideoConfigTag, get_provider_video_config};
use crate::api::videos_http_transport::video_config::{Headers, Params};

// the resolved provider, its config, the mapped options and the logging object's state, held for the handler call
pub(crate) struct Prepared<'call, Providers: VideoGenerationProviders> {
    pub(crate) model: String,
    pub(crate) provider: String,
    pub(crate) config: &'call Providers::Config,
    pub(crate) source: &'call Providers::Source,
    pub(crate) params: Params,
    pub(crate) extra_headers: Option<Headers>,
    pub(crate) extra_body: Option<Params>,
    pub(crate) timeout: Duration,
}

impl<Providers, Handler> VideoGenerationCaller<Providers, Handler>
where
    Providers: VideoGenerationProviders,
    Handler: VideoGenerationHandling<Providers::Config, Providers::Source>,
{
    // LiteLLM's try block from get_llm_provider to the handler call, every step in order
    pub(super) fn prepare<'call>(
        &'call self,
        request: &mut VideoGenerationRequest,
        logging: &mut LoggingState,
        resolution: &ResolutionContext<'_>,
    ) -> Result<Prepared<'call, Providers>, VideoGenerationFailure> {
        let resolved = self.resolve_target(request, resolution)?;
        let (model, provider) = (resolved.model, resolved.provider.slug().to_owned());
        let tag = self.video_tag(&model, &provider)?;
        let (config, source) = self.config_source(&tag, &model, &provider)?;
        let options = requested_optional_params(request)
            .map_err(|error| failure(error, Some(&model), Some(&provider), &self.redactor))?;
        let params = provider_request_params(config, &model, self.drop_params, &options);
        log_call(
            logging,
            &LoggedGeneration {
                kwargs: &request.kwargs,
                model: &model,
                user: request.user.as_deref(),
                params: &params,
                provider: &provider,
                call_id: call_id(&request.kwargs),
            },
        )
        .map_err(|error| {
            failure(
                VideoGenerationCallError::Logging(error),
                Some(&model),
                Some(&provider),
                &self.redactor,
            )
        })?;
        let timeout = resolved_timeout(request.timeout, self.request_timeout)
            .map_err(|error| failure(error, Some(&model), Some(&provider), &self.redactor))?;
        Ok(Prepared {
            model,
            provider,
            config,
            source,
            params,
            extra_headers: request.extra_headers.clone(),
            extra_body: request.extra_body.clone(),
            timeout,
        })
    }

    // get_llm_provider(model=model, custom_llm_provider=..., api_base=None, api_key=None)
    fn resolve_target(
        &self,
        request: &VideoGenerationRequest,
        resolution: &ResolutionContext<'_>,
    ) -> Result<ResolvedModel, VideoGenerationFailure> {
        let requested_model = request
            .model
            .as_deref()
            .filter(|model| !model.is_empty())
            .unwrap_or(DEFAULT_MODEL);
        resolve(
            &ResolutionRequest {
                model: requested_model,
                // an empty name reaches the resolver like an absent one, the same falsy check LiteLLM makes
                custom_llm_provider: request.custom_llm_provider.as_deref(),
                api_base: None,
                api_key: None,
            },
            resolution,
        )
        .map_err(|error| {
            failure(
                VideoGenerationCallError::Resolution(error),
                Some(requested_model),
                request.custom_llm_provider.as_deref(),
                &self.redactor,
            )
        })
    }

    // LlmProviders(...) and get_provider_video_config, whose failures get_llm_provider's treatment
    fn video_tag(
        &self,
        model: &str,
        provider: &str,
    ) -> Result<VideoConfigTag, VideoGenerationFailure> {
        let provider_id = ProviderId::from_str(provider).map_err(|source| {
            failure(
                VideoGenerationCallError::UnknownProvider {
                    provider: provider.to_owned(),
                    source,
                },
                Some(model),
                Some(provider),
                &self.redactor,
            )
        })?;
        match get_provider_video_config(Some(model), provider_id) {
            Ok(Some(tag)) => Ok(tag),
            Ok(None) => Err(failure(
                VideoGenerationCallError::Unsupported {
                    provider: provider.to_owned(),
                },
                Some(model),
                Some(provider),
                &self.redactor,
            )),
            Err(error) => Err(failure(
                VideoGenerationCallError::Selection(error),
                Some(model),
                Some(provider),
                &self.redactor,
            )),
        }
    }

    // the installed config for the tag; a provider LiteLLM has a config for but betterlite has not built yet
    fn config_source<'call>(
        &'call self,
        tag: &VideoConfigTag,
        model: &str,
        provider: &str,
    ) -> Result<(&'call Providers::Config, &'call Providers::Source), VideoGenerationFailure> {
        let Some((config, source)) = self.providers.video_config(tag) else {
            return Err(failure(
                VideoGenerationCallError::ConfigMissing {
                    provider: provider.to_owned(),
                },
                Some(model),
                Some(provider),
                &self.redactor,
            ));
        };
        Ok((config, source))
    }
}

// kwargs.get("litellm_call_id", None): the value rides into litellm_params, and only a string names a call
fn call_id(kwargs: &IndexMap<String, Json>) -> Option<&str> {
    match kwargs.get(LITELLM_CALL_ID) {
        Some(Json::Text(id)) => Some(id),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

// `timeout or DEFAULT_REQUEST_TIMEOUT`: an explicit zero or none takes B198's request timeout
fn resolved_timeout(
    timeout: Option<f64>,
    request_timeout: Duration,
) -> Result<Duration, VideoGenerationCallError> {
    match timeout {
        Some(seconds) if seconds != 0.0_f64 => Duration::try_from_secs_f64(seconds)
            .map_err(|source| VideoGenerationCallError::Timeout { seconds, source }),
        Some(_) | None => Ok(request_timeout),
    }
}
