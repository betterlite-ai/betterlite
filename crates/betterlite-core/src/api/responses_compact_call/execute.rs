use std::time::Duration;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_context::non_default_params::Params;
use betterlite_models::call_helpers::drop_params::normalize_drop_params;
use betterlite_models::controls::call_params::CallParams;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::response_events::ResponsesReply;
use betterlite_openai::response_input::{ResponsesInput, ResponsesItem};
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;

use crate::api::response_id_traversal::record::Record;
use crate::api::response_id_traversal::restore::restore_encrypted_item_ids;
use crate::api::responses_compact_http::config::CompactParams;
use crate::api::responses_compact_http::service::{
    ClientKind, ClientSource, CompactCall, CompactDeps, LogContext, prepare, send,
};
use crate::api::responses_parameters::optional::optional_params_responses;
use crate::api::responses_parameters::provider::{DropPolicy, ResponsesParamProvider};
use crate::api::responses_parameters::requested::requested_optional_params;
use crate::api::responses_provider_selection::ParameterMap;
use crate::api::responses_provider_selection::provider_config::{
    ConfigLookup, resolve_provider_config,
};
use crate::api::responses_provider_selection::resolution::{ModelResolver, resolve_model_provider};

use super::call::CompactResponsesCall;
use super::convert::{json_value, object_from_params, prompt_decision};
use super::deps::{CompactConfigFactory, CompactResponsesDeps};
use super::encode::encode_response;
use super::error::{CompactParamError, CompactResponsesError};
use super::logging;

pub(super) struct Completed {
    pub(super) reply: ResponsesReply,
    pub(super) provider: String,
}

pub(super) struct Resolved {
    pub(super) model: String,
    pub(super) provider: String,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<SecretString>,
    pub(super) local_vars: ParameterMap,
}

struct Pending<'call, P> {
    call: &'call CompactResponsesCall,
    config: &'call P,
    resolved: &'call Resolved,
    optional_params: &'call Object,
    params: CompactParams,
    timeout: Duration,
    mode: ClientKind,
}

// The compact wrapper's whole path: resolve, filter options, log, restore IDs, POST, then encode the reply
pub(super) async fn run<S, F>(
    deps: &CompactResponsesDeps<'_, S, F>,
    call: &CompactResponsesCall,
    custom_llm_provider: Option<&str>,
    mode: ClientKind,
    log: &mut LogContext<'_, '_>,
) -> Result<Completed, CompactResponsesError>
where
    S: ClientSource,
    F: CompactConfigFactory,
{
    let request_timeout = RequestTimeout::resolve(deps.environment)?;
    let resolved = resolve(deps.resolver, deps.lookup, call, custom_llm_provider)?;
    let config = config_for(deps.configs, &resolved)?;
    let filtered = options(deps.drop_params, call, &resolved, &config)?;
    let timeout = timeout_duration(call.timeout, request_timeout)?;
    logging::log_call(&mut *log.state, call, &resolved, &filtered)?;
    let optional_params = object_from_params(&filtered);
    let pending = Pending {
        call,
        config: &config,
        resolved: &resolved,
        optional_params: &optional_params,
        params: compact_params(call, &resolved),
        timeout,
        mode,
    };
    let mut reply = execute(deps.source, &pending, log).await?;
    encode_response(&mut reply, call, &resolved.provider)?;
    Ok(Completed {
        reply,
        provider: resolved.provider,
    })
}

// The preliminary lookup the async wrapper runs for exception mapping gets no base and no credentials
pub(super) fn preliminary_provider(
    resolver: &dyn ModelResolver,
    call: &CompactResponsesCall,
) -> Result<String, CompactResponsesError> {
    let mut params = CallParams::default();
    let mut local_vars = ParameterMap::new();
    let (_, provider) =
        resolve_model_provider(resolver, &call.model, None, &mut params, &mut local_vars)?;
    Ok(provider)
}

fn resolve(
    resolver: &dyn ModelResolver,
    lookup: &dyn ConfigLookup,
    call: &CompactResponsesCall,
    custom_llm_provider: Option<&str>,
) -> Result<Resolved, CompactResponsesError> {
    let mut params = CallParams {
        api_key: call.api_key.clone(),
        api_base: call.api_base.clone(),
        ..CallParams::default()
    };
    let mut local_vars = ParameterMap::new();
    let (model, provider) = resolve_model_provider(
        resolver,
        &call.model,
        custom_llm_provider,
        &mut params,
        &mut local_vars,
    )?;
    if provider.is_empty() {
        return Err(CompactResponsesError::MissingProvider);
    }
    supported(lookup, &model, &provider, params.api_base.as_deref())?;
    Ok(Resolved {
        model,
        provider,
        api_base: params.api_base,
        api_key: params.api_key,
        local_vars,
    })
}

// B157: a provider without a responses config has no compact handler
fn supported(
    lookup: &dyn ConfigLookup,
    model: &str,
    provider: &str,
    api_base: Option<&str>,
) -> Result<(), CompactResponsesError> {
    if resolve_provider_config(lookup, model, provider, &JsonValue::Null, api_base).is_none() {
        return Err(CompactResponsesError::Unsupported {
            provider: provider.to_owned(),
        });
    }
    Ok(())
}

fn config_for<F>(configs: &F, resolved: &Resolved) -> Result<F::Provider, CompactResponsesError>
where
    F: CompactConfigFactory,
{
    configs
        .compact_config(
            &resolved.model,
            &resolved.provider,
            resolved.api_base.as_deref(),
        )
        .ok_or_else(|| CompactResponsesError::Unsupported {
            provider: resolved.provider.clone(),
        })
}

// B905 with allowed_openai_params fixed to null, and the request's own drop_params normalized by B206
fn options<P>(
    global_drop_params: bool,
    call: &CompactResponsesCall,
    resolved: &Resolved,
    provider: &P,
) -> Result<Params, CompactResponsesError>
where
    P: ResponsesParamProvider<Error = CompactParamError>,
{
    let mut requested = requested_options(call, resolved)?;
    let policy = DropPolicy {
        global: global_drop_params,
        call: normalize_drop_params(call.kwargs.get("drop_params")),
    };
    optional_params_responses(provider, &resolved.model, &mut requested, &[], policy)
        .map_err(CompactResponsesError::from)
}

// The arguments the compact handler sends into get_optional_params; extra_query never joins them
fn requested_options(
    call: &CompactResponsesCall,
    resolved: &Resolved,
) -> Result<Params, CompactResponsesError> {
    let mut optional: Params = resolved
        .local_vars
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    if let Some(instructions) = &call.instructions {
        optional.insert(
            String::from("instructions"),
            JsonValue::Text(instructions.clone()),
        );
    }
    if let Some(previous) = &call.previous_response_id {
        optional.insert(
            String::from("previous_response_id"),
            JsonValue::Text(previous.clone()),
        );
    }
    if !call.kwargs.is_empty() {
        optional.insert(
            String::from("kwargs"),
            JsonValue::Object(
                call.kwargs
                    .iter()
                    .map(|(name, value)| (name.clone(), json_value(value)))
                    .collect(),
            ),
        );
    }
    requested_optional_params(&mut optional)
        .map_err(CompactParamError::from)
        .map_err(CompactResponsesError::from)
}

fn compact_params(call: &CompactResponsesCall, resolved: &Resolved) -> CompactParams {
    CompactParams {
        api_base: resolved.api_base.clone(),
        api_key: resolved.api_key.clone(),
        ssl_verify: call.ssl_verify,
    }
}

// `timeout or request_timeout`: zero falls back, and the fallback reads REQUEST_TIMEOUT with its 6000s default
fn timeout_duration(
    configured: Option<f64>,
    request_timeout: RequestTimeout,
) -> Result<Duration, CompactResponsesError> {
    let seconds = configured
        .filter(|value| *value != 0.0_f64)
        .unwrap_or(request_timeout.seconds);
    Duration::try_from_secs_f64(seconds)
        .map_err(|_error| CompactResponsesError::InvalidTimeout { seconds })
}

// B904 decodes the litellm-encoded item IDs and wrapped content before the request reaches the provider
fn restored(input: &ResponsesInput) -> ResponsesInput {
    let ResponsesInput::Items(items) = input else {
        return input.clone();
    };
    let mut record = Record::List(
        items
            .iter()
            .map(|item| Record::from_json(prompt_decision(&item.0)))
            .collect(),
    );
    restore_encrypted_item_ids(&mut record);
    let Json::List(entries) = record.into_json() else {
        return input.clone();
    };
    ResponsesInput::Items(entries.into_iter().map(ResponsesItem).collect())
}

async fn execute<S, F>(
    source: &S,
    pending: &Pending<'_, F::Provider>,
    log: &mut LogContext<'_, '_>,
) -> Result<ResponsesReply, CompactResponsesError>
where
    S: ClientSource,
    F: CompactConfigFactory,
{
    let http = CompactDeps {
        config: pending.config,
        source,
        supplied: None,
    };
    let input = restored(&pending.call.input);
    let compact = CompactCall {
        model: &pending.resolved.model,
        input: &input,
        optional_params: pending.optional_params,
        params: &pending.params,
        extra_headers: pending.call.extra_headers.as_ref(),
        extra_body: pending.call.extra_body.as_ref(),
        timeout: Some(pending.timeout),
        shared_session: pending.call.shared_session,
    };
    let prepared = prepare(&http, pending.mode, &compact, log).await?;
    send(prepared, log)
        .await
        .map_err(CompactResponsesError::from)
}
