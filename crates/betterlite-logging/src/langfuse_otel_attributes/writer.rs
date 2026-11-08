use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use opentelemetry::trace::Span;

use crate::langfuse_projections::steering::truthiness;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::{LLMObsAttributes as _, SafeSetAttribute as _};
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};

use super::keys::{LangfuseSpanAttributes, OBSERVATION_TYPE};
use super::llm_obs::LangfuseLLMObsOTELAttributes;
use super::metadata::{extract_langfuse_metadata, set_metadata_attributes};
use super::observation::set_observation_output;
use super::wire::{field, json_from_metadata, json_value, object, text};

// litellm.integrations.langfuse.langfuse_otel.LangfuseOtelLogger.set_langfuse_otel_attributes
pub fn set_langfuse_otel_attributes<S>(
    span: &mut S,
    kwargs: &MetadataMap,
    response: Option<&OtelModelValue>,
    environment: &dyn EnvironmentLookup,
    _redact_messages: bool,
) where
    S: Span,
{
    LangfuseLLMObsOTELAttributes::set_messages(span, kwargs);
    span.safe_set_attribute(
        OBSERVATION_TYPE,
        &OtelModelValue::String("generation".to_owned()),
    );
    let kwargs_json = json_from_metadata(kwargs);
    let response_json = response.map(json_value);
    set_langfuse_extra_attributes(span, &kwargs_json, response_json.as_ref(), environment);
}

// litellm.integrations.langfuse.langfuse_otel.LangfuseOtelLogger._set_langfuse_specific_attributes
fn set_langfuse_extra_attributes<S>(
    span: &mut S,
    kwargs: &Json,
    response: Option<&Json>,
    environment: &dyn EnvironmentLookup,
) where
    S: Span,
{
    let dynamic_environment = field(kwargs, "standard_callback_dynamic_params")
        .and_then(object)
        .and_then(|params| params.get("langfuse_environment"))
        .filter(|value| !matches!(value, Json::Null))
        .and_then(text);
    let resolved_environment = dynamic_environment.map(str::to_owned).or_else(|| {
        environment
            .lookup("LANGFUSE_TRACING_ENVIRONMENT")
            .filter(|value| !value.is_empty())
    });
    if let Some(value) = resolved_environment {
        span.safe_set_attribute(
            LangfuseSpanAttributes::LangfuseEnvironment.value(),
            &OtelModelValue::String(value),
        );
    }
    let metadata = extract_langfuse_metadata(kwargs);
    set_metadata_attributes(span, &metadata);
    if let Some(messages) = field(kwargs, "messages").filter(|value| truthiness(value))
        && let Ok(record) = safe_dumps(messages, DEFAULT_MAX_RECURSE_DEPTH, None)
    {
        span.safe_set_attribute(
            LangfuseSpanAttributes::ObservationInput.value(),
            &OtelModelValue::String(record),
        );
    }
    set_observation_output(span, response);
}
