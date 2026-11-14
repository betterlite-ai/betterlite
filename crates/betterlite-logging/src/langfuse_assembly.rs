pub mod generation;
pub mod metadata;
pub mod observation;
pub mod trace_ids;
pub mod trace_params;
pub mod usage;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use opentelemetry::trace::{TraceId, Tracer};
use opentelemetry_sdk::trace::IdGenerator;
use rust_decimal::Decimal;

use crate::fields::payload::StandardLoggingPayload;
use crate::fields::usage::UsageError;
use crate::langfuse_observations::identity::{Timestamp, resolve_trace_id};
use crate::langfuse_rest::client::LangfuseApiClient;

use self::observation::ObservationRequest;
use self::trace_ids::CallTrace;
use self::trace_params::TracePlanRequest;

// the injected langfuse_masking_function; Sync so the assembly future can cross threads
pub type Masker<'masker> = &'masker (dyn Fn(&Json) -> Json + Sync);

pub const REDACTED_BY_LITELLM: &str = "redacted-by-litellm";

// the per-call facts _log_langfuse_v2 reads out of kwargs, litellm_params and the response object
pub struct GenerationTrace<'trace> {
    pub call_type: &'trace str,
    pub level: &'trace str,
    pub model: &'trace str,
    pub provider: Option<&'trace str>,
    pub metadata: &'trace IndexMap<String, Json>,
    pub litellm_params: Option<&'trace Json>,
    pub payload: Option<&'trace StandardLoggingPayload>,
    pub response: Option<&'trace Json>,
    pub input: Option<&'trace Json>,
    pub output: Option<&'trace Json>,
    pub optional_params: IndexMap<String, Json>,
    pub litellm_call_id: Option<&'trace str>,
    pub litellm_trace_id: Option<&'trace str>,
    pub start_time: Option<Timestamp>,
    pub end_time: Option<Timestamp>,
    pub completion_start_time: Option<Timestamp>,
    pub response_cost: Option<Decimal>,
    pub vertex_location: Option<&'trace str>,
    pub aws_region_name: Option<&'trace str>,
    pub cache_hit: Option<bool>,
    pub preset_cache_key: Option<&'trace str>,
    pub masker: Option<Masker<'trace>>,
}

// the litellm_settings values _log_langfuse_v2 reads while it assembles
pub struct AssemblySettings<'settings> {
    pub default_tags: &'settings [String],
    pub update_trace_keys: bool,
    pub redact_user_api_key_info: bool,
}

// the observation services the assembly starts spans through, injected so tests own their tracer
pub struct AssemblyRuntime<'runtime, T>
where
    T: Tracer,
{
    pub tracer: &'runtime T,
    pub id_generator: &'runtime dyn IdGenerator,
    pub prompt_client: &'runtime LangfuseApiClient,
    pub environment: &'runtime (dyn EnvironmentLookup + Sync),
}

#[derive(Clone, Copy)]
pub struct AssemblyInput<'input> {
    pub trace: &'input GenerationTrace<'input>,
    pub settings: &'input AssemblySettings<'input>,
}

#[derive(Debug, thiserror::Error)]
pub enum AssemblyError {
    #[error("the standard logging payload could not be written as JSON")]
    Payload(#[source] serde_json::Error),
    #[error("the response usage could not be read")]
    Usage(#[source] UsageError),
    #[error("the prompt management metadata could not be read")]
    PromptMetadata(#[source] serde_json::Error),
}

// litellm.integrations.langfuse.langfuse.LangFuseLogger._log_langfuse_v2; a failure anywhere returns (None, None)
#[must_use]
pub async fn assemble_generation_trace<T>(
    trace: &GenerationTrace<'_>,
    runtime: &AssemblyRuntime<'_, T>,
    settings: &AssemblySettings<'_>,
) -> (Option<TraceId>, Option<String>)
where
    T: Tracer + Sync,
{
    match assemble(trace, runtime, settings).await {
        Ok(logged) => logged,
        Err(error) => {
            tracing::error!("Langfuse Layer Error - {error}");
            (None, None)
        }
    }
}

async fn assemble<T>(
    trace: &GenerationTrace<'_>,
    runtime: &AssemblyRuntime<'_, T>,
    settings: &AssemblySettings<'_>,
) -> Result<(Option<TraceId>, Option<String>), AssemblyError>
where
    T: Tracer + Sync,
{
    let input = AssemblyInput { trace, settings };
    let mut cleaned = metadata::clean(&input);
    let call = trace_ids::resolve(trace, &cleaned.steering);
    debug_session_reuse(&call);
    let masked = metadata::mask(trace.masker, trace.input, trace.output);
    let plan = trace_params::build(
        &TracePlanRequest {
            input: &input,
            steering: &cleaned.steering,
            call: &call,
            masked: &masked,
            environment: runtime.environment,
        },
        &mut cleaned.fields,
        &mut cleaned.tags,
    );
    let params = generation::build(
        &input,
        &cleaned.steering,
        &masked,
        &mut cleaned.fields,
        runtime.prompt_client,
    )
    .await?;
    let trace_id = resolve_trace_id(call.call_trace_id.as_deref(), runtime.id_generator);
    let generation_id = observation::observe(&ObservationRequest {
        trace,
        call: &call,
        params: &params,
        plan: &plan,
        runtime,
        trace_id,
    });
    Ok((Some(trace_id), Some(generation_id)))
}

// the source logs when a session header's trace id gives way to this call's own
fn debug_session_reuse(call: &CallTrace) {
    if call.call_trace_id != call.trace_id {
        tracing::debug!(
            "Langfuse: trace id {:?} came from a session header; using call id {:?} so each call gets its own trace",
            call.trace_id,
            call.call_trace_id
        );
    }
}

// litellm_params["proxy_server_request"], which carries the method, url and headers of the proxy call
fn proxy_request(litellm_params: Option<&Json>) -> Option<&Json> {
    match litellm_params {
        Some(Json::Object(fields)) => fields.get("proxy_server_request"),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}

// a text litellm_params value such as api_base, which the trace metadata enrichment carries
fn params_text<'params>(litellm_params: Option<&'params Json>, name: &str) -> Option<&'params str> {
    let Some(Json::Object(fields)) = litellm_params else {
        return None;
    };
    match fields.get(name) {
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

#[path = "langfuse_assembly/tests/assemble.rs"]
mod assemble_tests;
#[path = "langfuse_assembly/tests/generation.rs"]
mod generation_tests;
#[path = "langfuse_assembly/tests/metadata.rs"]
mod metadata_tests;
#[path = "langfuse_assembly/tests/support.rs"]
mod support;
#[path = "langfuse_assembly/tests/trace_ids.rs"]
mod trace_ids_tests;
