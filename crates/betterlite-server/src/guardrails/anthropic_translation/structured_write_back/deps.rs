use std::convert::Infallible;
use std::future::{Future, ready};

use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_models::model_capability_predicates::{CapabilityPredicates, CapabilityRequest};
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;

// The write-back has no fetcher, so a remote media URL fails instead of reaching the network.
pub(super) struct NoMedia;

impl RemoteMediaFetcher for NoMedia {
    fn convert_url(
        &self,
        url: &str,
    ) -> impl Future<Output = Result<String, ImageFetchError>> + Send {
        ready(Err(ImageFetchError::Disabled {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> + Send {
        ready(Err(ImageFetchError::Disabled {
            url: urls.first().cloned().unwrap_or_default(),
        }))
    }
}

// The write-back asks no parameter or capability question; the adapter's mid-turn checks run first.
pub(super) struct NoParameters;

impl SupportedParameterSource for NoParameters {
    type Error = Infallible;

    fn supported_openai_params(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Result<Option<Vec<String>>, Infallible> {
        Ok(None)
    }
}

pub(super) struct NoCapabilities;

impl CapabilityPredicates for NoCapabilities {
    fn supports_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn declared_value_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_native_streaming(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = Json> + Send {
        ready(Json::Null)
    }

    fn supports_response_schema(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }
}
