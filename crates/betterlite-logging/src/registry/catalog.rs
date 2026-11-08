pub mod agentops;
pub mod argilla;
pub mod arize;
pub mod arize_phoenix;
pub mod athina;
pub mod aws_sqs;
pub mod azure_sentinel;
pub mod azure_storage;
pub mod bitbucket;
pub mod braintrust;
pub mod clickhouse;
pub mod cloudzero;
pub mod datadog_cost_management;
pub mod datadog_llm_observability;
pub mod datadog_metrics;
pub mod deepeval;
pub mod dotprompt;
pub mod dynamodb;
pub mod focus;
pub mod galileo;
pub mod gcs_bucket;
pub mod gcs_pubsub;
pub mod gitlab;
pub mod greenscale;
pub mod helicone;
pub mod humanloop;
pub mod lago;
pub mod langfuse;
pub mod langfuse_otel;
pub mod langsmith;
pub mod langtrace;
pub mod levo;
pub mod literalai;
pub mod logfire;
pub mod mavvrik;
pub mod newrelic;
pub mod openmeter;
pub mod opik;
pub mod otel;
pub mod pointfive;
pub mod posthog;
pub mod promptlayer;
pub mod s3;
pub mod s3_v2;
pub mod signoz;
pub mod supabase;
pub mod traceloop;
pub mod vantage;
pub mod weave_otel;
pub mod zerobus;
use super::error::RegistryError;
use super::sink::CallbackSink;
use crate::callback_config::error::ValidationError;
use crate::callback_config::family::{conflicting_span_scope_error, cross_entry_family_error};
use crate::callback_config::options::callback_config_error;
use crate::callback_config::{CallbackVars, FeatureFlags};

pub fn validate_callback_vars(
    callback_name: Option<&str>,
    vars: &CallbackVars,
    stored: &[CallbackVars],
    flags: FeatureFlags,
) -> Result<(), ValidationError> {
    callback_config_error(callback_name, vars, flags)?;
    cross_entry_family_error(vars, stored)?;
    conflicting_span_scope_error(vars, stored)
}

// Every LiteLLM name is unavailable until M5 compiles its exporter in; none resolves to a sink
pub fn adapter_for(name: &str) -> Result<CallbackSink, RegistryError> {
    Err(RegistryError::UnavailableAdapter {
        name: name.to_owned(),
    })
}
