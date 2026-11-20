use std::collections::BTreeMap;

use opentelemetry_sdk::trace::SdkTracer;

use crate::otel::legacy_exporters::headers::{HeadersInput, headers_dictionary};
use crate::otel::legacy_policy::config::OpenTelemetryConfig;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;

use super::error::LegacyLifecycleError;
use super::lifecycle::LegacyLifecycle;

pub const LANGFUSE_OTEL: &str = "langfuse_otel";

// the vendor subclasses override these two hooks; the base class routes nothing
pub trait DynamicOtel: Send + Sync {
    fn construct_dynamic_otel_headers(
        &self,
        dynamic_params: &MetadataMap,
    ) -> Option<BTreeMap<String, String>>;

    fn construct_dynamic_otel_config(
        &self,
        dynamic_params: &MetadataMap,
    ) -> Option<OpenTelemetryConfig>;
}

// a callback that sends every span of a request to a provider chosen per request, such as Phoenix's projects
pub trait TracerRouter: Send + Sync {
    fn route(
        &self,
        lifecycle: &LegacyLifecycle,
        kwargs: &MetadataMap,
    ) -> Result<Option<SdkTracer>, LegacyLifecycleError>;
}

#[derive(Debug, Default)]
pub struct BaseDynamicOtel;

impl DynamicOtel for BaseDynamicOtel {
    fn construct_dynamic_otel_headers(
        &self,
        _dynamic_params: &MetadataMap,
    ) -> Option<BTreeMap<String, String>> {
        None
    }

    fn construct_dynamic_otel_config(
        &self,
        _dynamic_params: &MetadataMap,
    ) -> Option<OpenTelemetryConfig> {
        None
    }
}

#[derive(Clone, Debug)]
pub enum TracerChoice {
    DynamicConfig(Box<OpenTelemetryConfig>),
    DynamicHeaders(BTreeMap<String, String>),
    EnvHeaders(BTreeMap<String, String>),
    Global,
}

impl TracerChoice {
    #[must_use]
    pub const fn branch(&self) -> &'static str {
        match self {
            Self::DynamicConfig(_) => "dynamic_config",
            Self::DynamicHeaders(_) => "dynamic_headers",
            Self::EnvHeaders(_) => "env_headers",
            Self::Global => "global",
        }
    }
}

// LiteLLM tests the dynamic params for truthiness; an absent or empty bucket is unset, any other shape is an error
pub fn dynamic_params(kwargs: &MetadataMap) -> Result<Option<&MetadataMap>, LegacyLifecycleError> {
    match kwargs.get("standard_callback_dynamic_params") {
        None | Some(OtelModelValue::Null(())) => Ok(None),
        Some(OtelModelValue::Object(fields)) if fields.is_empty() => Ok(None),
        Some(OtelModelValue::Object(fields)) => Ok(Some(fields)),
        Some(_) => Err(LegacyLifecycleError::DynamicParams),
    }
}

pub fn tracer_choice(
    config: &OpenTelemetryConfig,
    callback_name: Option<&str>,
    dynamic: &dyn DynamicOtel,
    kwargs: &MetadataMap,
) -> Result<TracerChoice, LegacyLifecycleError> {
    if let Some(params) = dynamic_params(kwargs)? {
        if let Some(dynamic_config) = dynamic.construct_dynamic_otel_config(params) {
            return Ok(TracerChoice::DynamicConfig(Box::new(dynamic_config)));
        }
        if let Some(headers) = dynamic
            .construct_dynamic_otel_headers(params)
            .filter(|headers| !headers.is_empty())
        {
            return Ok(TracerChoice::DynamicHeaders(headers));
        }
    }
    if callback_name == Some(LANGFUSE_OTEL) {
        let env_var_headers = headers_dictionary(
            config
                .headers
                .as_deref()
                .map_or(HeadersInput::None, HeadersInput::Text),
        )?;
        if !env_var_headers.is_empty() {
            return Ok(TracerChoice::EnvHeaders(env_var_headers));
        }
    }
    Ok(TracerChoice::Global)
}

pub(crate) fn tracer_for_request(
    lifecycle: &LegacyLifecycle,
    kwargs: &MetadataMap,
) -> Result<SdkTracer, LegacyLifecycleError> {
    if let Some(router) = &lifecycle.tracer_router
        && let Some(tracer) = router.route(lifecycle, kwargs)?
    {
        return Ok(tracer);
    }
    match lifecycle.tracer_choice(kwargs)? {
        TracerChoice::DynamicConfig(config) => {
            Ok(lifecycle.tracer_cache.tracer_for_config(&config)?)
        }
        TracerChoice::DynamicHeaders(headers) | TracerChoice::EnvHeaders(headers) => {
            Ok(lifecycle.tracer_cache.tracer_for_headers(&headers)?)
        }
        TracerChoice::Global => Ok(lifecycle.tracer.clone()),
    }
}
